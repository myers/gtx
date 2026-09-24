//! `--template`: gh's Go-template output formatting.
//!
//! The engine is the `gotmpl` crate (a port of Go's `text/template`), fed
//! the same JSON `--json` would print and given gh's helper functions:
//! `tablerow`, `tablerender`, `timeago`, `timefmt`, `truncate`, `color`,
//! `autocolor`, `join`, `pluck` and `hyperlink`.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use eyre::Result;
use gotmpl::TemplateError;
use gotmpl::{Template, Value};
use unicode_width::UnicodeWidthStr;

type FnResult = std::result::Result<Value, TemplateError>;

/// Render `src` over `data` and print it, the way `gh --template` does
/// (no newline is added).
pub fn print(src: &str, data: &serde_json::Value) -> Result<()> {
    let color = std::io::IsTerminal::is_terminal(&std::io::stdout())
        && std::env::var_os("NO_COLOR").is_none();
    let out = render(src, data, color, Utc::now())?;
    print!("{out}");
    Ok(())
}

/// Output so far plus the pending `tablerow` rows, shared between the
/// template's writer and the table helpers so a table lands where
/// `tablerender` (or the end of the template) puts it.
#[derive(Default)]
struct State {
    out: String,
    rows: Vec<Vec<String>>,
}

impl State {
    fn render_table(&mut self) {
        let rows = std::mem::take(&mut self.rows);
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0; cols];
        for row in &rows {
            for (i, f) in row.iter().enumerate() {
                widths[i] = widths[i].max(f.width());
            }
        }
        for row in &rows {
            for (i, f) in row.iter().enumerate() {
                if i > 0 {
                    self.out.push_str("  ");
                }
                self.out.push_str(f);
                if i + 1 < cols {
                    self.out.push_str(&" ".repeat(widths[i] - f.width()));
                }
            }
            self.out.push('\n');
        }
    }
}

struct Sink(Arc<Mutex<State>>);

impl std::fmt::Write for Sink {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.lock().map_err(|_| std::fmt::Error)?.out.push_str(s);
        Ok(())
    }
}

fn err(msg: impl Into<String>) -> TemplateError {
    TemplateError::Exec(msg.into())
}

fn arg<'a>(
    name: &str,
    args: &'a [Value],
    i: usize,
) -> std::result::Result<&'a Value, TemplateError> {
    args.get(i)
        .ok_or_else(|| err(format!("{name}: missing argument")))
}

fn str_arg<'a>(
    name: &str,
    args: &'a [Value],
    i: usize,
) -> std::result::Result<&'a str, TemplateError> {
    arg(name, args, i)?
        .as_str()
        .ok_or_else(|| err(format!("{name}: expected a string, got {}", args[i])))
}

/// gh's `jsonScalarToString`: how template helpers turn a JSON scalar into text.
fn scalar(v: &Value) -> std::result::Result<String, TemplateError> {
    match v {
        Value::String(s) => Ok(s.to_string()),
        Value::Int(n) => Ok(n.to_string()),
        Value::Uint(n) => Ok(n.to_string()),
        Value::Float(f) if f.trunc() == *f => Ok(format!("{f:.0}")),
        Value::Float(f) => Ok(format!("{f:.2}")),
        Value::Nil => Ok(String::new()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Err(err(format!("cannot convert type to string: {other}"))),
    }
}

fn to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::Uint(u)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::String(s.as_str().into()),
        serde_json::Value::Array(a) => Value::List(a.iter().map(to_value).collect()),
        serde_json::Value::Object(o) => Value::Map(Arc::new(
            o.iter()
                .map(|(k, v)| (k.as_str().into(), to_value(v)))
                .collect(),
        )),
    }
}

fn parse_time(name: &str, s: &str) -> std::result::Result<DateTime<FixedOffset>, TemplateError> {
    DateTime::parse_from_rfc3339(s)
        .map_err(|e| err(format!("{name}: cannot parse time {s:?}: {e}")))
}

/// Render `src` over `data`. `color` is whether `autocolor` colors; `now` is
/// what `timeago` measures from.
pub fn render(
    src: &str,
    data: &serde_json::Value,
    color: bool,
    now: DateTime<Utc>,
) -> Result<String> {
    let state = Arc::new(Mutex::new(State::default()));
    let row_state = state.clone();
    let render_state = state.clone();
    let tmpl = Template::new("")
        .func("tablerow", move |args| {
            let row = args
                .iter()
                .map(scalar)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            row_state
                .lock()
                .map_err(|_| err("tablerow: poisoned"))?
                .rows
                .push(row);
            Ok(Value::String("".into()))
        })
        .func("tablerender", move |_| {
            render_state
                .lock()
                .map_err(|_| err("tablerender: poisoned"))?
                .render_table();
            Ok(Value::String("".into()))
        })
        .func("timeago", move |args| {
            let t = parse_time("timeago", str_arg("timeago", args, 0)?)?;
            Ok(Value::String(time_ago(now, t.with_timezone(&Utc)).into()))
        })
        .func("timefmt", |args| {
            let layout = str_arg("timefmt", args, 0)?;
            let t = parse_time("timefmt", str_arg("timefmt", args, 1)?)?;
            Ok(Value::String(go_time_format(layout, &t).into()))
        })
        .func("truncate", |args| {
            let n = arg("truncate", args, 0)?
                .as_int()
                .ok_or_else(|| err("truncate: expected an integer width"))?;
            let s = scalar(arg("truncate", args, 1)?)?;
            Ok(Value::String(
                truncate(usize::try_from(n).unwrap_or(0), &s).into(),
            ))
        })
        .func("color", |args| -> FnResult {
            let style = str_arg("color", args, 0)?;
            Ok(Value::String(
                ansi_color(&scalar(arg("color", args, 1)?)?, style).into(),
            ))
        })
        .func("autocolor", move |args| -> FnResult {
            let style = str_arg("autocolor", args, 0)?;
            let text = scalar(arg("autocolor", args, 1)?)?;
            Ok(Value::String(
                if color {
                    ansi_color(&text, style)
                } else {
                    text
                }
                .into(),
            ))
        })
        .func("join", |args| {
            let sep = str_arg("join", args, 0)?;
            let Value::List(items) = arg("join", args, 1)? else {
                return Err(err("join: expected a list"));
            };
            let parts = items
                .iter()
                .map(scalar)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Value::String(parts.join(sep).into()))
        })
        .func("pluck", |args| {
            let field = str_arg("pluck", args, 0)?;
            let Value::List(items) = arg("pluck", args, 1)? else {
                return Err(err("pluck: expected a list"));
            };
            let plucked = items
                .iter()
                .map(|item| match item {
                    Value::Map(m) => Ok(m.get(field).cloned().unwrap_or(Value::Nil)),
                    other => Err(err(format!("pluck: expected an object, got {other}"))),
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Value::List(plucked.into()))
        })
        .func("hyperlink", |args| {
            let link = str_arg("hyperlink", args, 0)?;
            let text = args
                .get(1)
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .unwrap_or(link);
            Ok(Value::String(
                format!("\x1b]8;;{link}\x1b\\{text}\x1b]8;;\x1b\\").into(),
            ))
        })
        .parse(src)
        .map_err(|e| eyre::eyre!("{e}"))?;

    tmpl.execute_fmt(&mut Sink(state.clone()), &to_value(data))
        .map_err(|e| eyre::eyre!("{e}"))?;
    let mut st = state
        .lock()
        .map_err(|_| eyre::eyre!("template state poisoned"))?;
    st.render_table();
    Ok(std::mem::take(&mut st.out))
}

/// gh's `text.RelativeTimeAgo`.
fn time_ago(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let ago = now - t;
    let about =
        |n: i64, unit: &str| format!("about {n} {unit}{} ago", if n == 1 { "" } else { "s" });
    let hours = ago.num_hours();
    if ago.num_minutes() < 1 {
        "less than a minute ago".into()
    } else if hours < 1 {
        about(ago.num_minutes(), "minute")
    } else if hours < 24 {
        about(hours, "hour")
    } else if hours < 30 * 24 {
        about(hours / 24, "day")
    } else if hours < 365 * 24 {
        about(hours / 24 / 30, "month")
    } else {
        about(hours / 24 / 365, "year")
    }
}

/// gh's `text.Truncate`: cut to `max` display columns, ending in `...`
/// when there's room for it.
fn truncate(max: usize, s: &str) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let tail = if max >= 5 { "..." } else { "" };
    let budget = max - tail.len();
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push_str(tail);
    if out.width() < max {
        out.push(' ');
    }
    out
}

/// `github.com/mgutz/ansi`'s `Color(s, style)`, which gh's `color` uses:
/// style is `fg[+attrs][:bg[+attrs]]`, colors by name or 256-color number.
fn ansi_color(s: &str, style: &str) -> String {
    if style.is_empty() {
        return s.to_string();
    }
    fn code(name: &str) -> u32 {
        match name {
            "red" => 1,
            "green" => 2,
            "yellow" => 3,
            "blue" => 4,
            "magenta" => 5,
            "cyan" => 6,
            "white" => 7,
            "default" => 9,
            _ => 0,
        }
    }
    let (fg, bg) = style.split_once(':').unwrap_or((style, ""));
    let (fg_key, fg_style) = fg.split_once('+').unwrap_or((fg, ""));
    let (bg_key, bg_style) = bg.split_once('+').unwrap_or((bg, ""));
    let mut c = String::from("\x1b[0;");
    for (flag, seq) in [
        ('b', "1;"),
        ('B', "5;"),
        ('u', "4;"),
        ('i', "7;"),
        ('s', "9;"),
    ] {
        if fg_style.contains(flag) {
            c.push_str(seq);
        }
    }
    let base = if fg_style.contains('h') { 90 } else { 30 };
    match fg_key.parse::<u32>() {
        Ok(n) => write!(c, "38;5;{n};").ok(),
        Err(_) => write!(c, "{};", base + code(fg_key)).ok(),
    };
    if !bg_key.is_empty() {
        let base = if bg_style.contains('h') { 100 } else { 40 };
        match bg_key.parse::<u32>() {
            Ok(n) => write!(c, "48;5;{n};").ok(),
            Err(_) => write!(c, "{};", base + code(bg_key)).ok(),
        };
    }
    c.pop();
    format!("{c}m{s}\x1b[0m")
}

/// Format `t` with a Go time layout (`2006-01-02 15:04:05 -0700 MST`).
fn go_time_format(layout: &str, t: &DateTime<FixedOffset>) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let month = MONTHS[t.month0() as usize];
    let day = DAYS[t.weekday().num_days_from_sunday() as usize];
    let h12 = match t.hour() % 12 {
        0 => 12,
        h => h,
    };
    let offset = t.offset().local_minus_utc();
    let zone = |z: bool, colon: bool, secs: bool, hours_only: bool| {
        if z && offset == 0 {
            return "Z".to_string();
        }
        let sign = if offset < 0 { '-' } else { '+' };
        let o = offset.abs();
        let (hh, mm, ss) = (o / 3600, o / 60 % 60, o % 60);
        let sep = if colon { ":" } else { "" };
        let mut s = format!("{sign}{hh:02}");
        if !hours_only {
            s += &format!("{sep}{mm:02}");
            if secs {
                s += &format!("{sep}{ss:02}");
            }
        }
        s
    };

    let b = layout.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < layout.len() {
        let rest = &layout[i..];
        let tokens: &[(&str, String)] = &[
            ("January", month.to_string()),
            ("Jan", month[..3].to_string()),
            ("Monday", day.to_string()),
            ("Mon", day[..3].to_string()),
            (
                "MST",
                if offset == 0 {
                    "UTC".into()
                } else {
                    zone(false, false, false, false)
                },
            ),
            ("2006", format!("{:04}", t.year())),
            ("002", format!("{:03}", t.ordinal())),
            ("__2", format!("{:>3}", t.ordinal())),
            ("_2", format!("{:>2}", t.day())),
            ("01", format!("{:02}", t.month())),
            ("02", format!("{:02}", t.day())),
            ("03", format!("{h12:02}")),
            ("04", format!("{:02}", t.minute())),
            ("05", format!("{:02}", t.second())),
            ("06", format!("{:02}", t.year() % 100)),
            ("15", format!("{:02}", t.hour())),
            ("1", format!("{}", t.month())),
            ("2", format!("{}", t.day())),
            ("3", format!("{h12}")),
            ("4", format!("{}", t.minute())),
            ("5", format!("{}", t.second())),
            ("PM", if t.hour() < 12 { "AM" } else { "PM" }.into()),
            ("pm", if t.hour() < 12 { "am" } else { "pm" }.into()),
            ("-07:00:00", zone(false, true, true, false)),
            ("-070000", zone(false, false, true, false)),
            ("-07:00", zone(false, true, false, false)),
            ("-0700", zone(false, false, false, false)),
            ("-07", zone(false, false, false, true)),
            ("Z07:00:00", zone(true, true, true, false)),
            ("Z070000", zone(true, false, true, false)),
            ("Z07:00", zone(true, true, false, false)),
            ("Z0700", zone(true, false, false, false)),
            ("Z07", zone(true, false, false, true)),
        ];
        if let Some((tok, val)) = tokens.iter().find(|(tok, _)| rest.starts_with(tok)) {
            out.push_str(val);
            i += tok.len();
            continue;
        }
        // Fractional seconds: `.000` / `,000` (fixed) or `.999` (trailing zeros cut).
        if (b[i] == b'.' || b[i] == b',')
            && i + 1 < b.len()
            && (b[i + 1] == b'0' || b[i + 1] == b'9')
        {
            let digit = b[i + 1];
            let mut j = i + 1;
            while j < b.len() && b[j] == digit {
                j += 1;
            }
            if j >= b.len() || !b[j].is_ascii_digit() {
                let n = j - i - 1;
                let frac = format!("{:09}", t.nanosecond() % 1_000_000_000);
                let mut digits = frac[..n.min(9)].to_string();
                if digit == b'9' {
                    digits = digits.trim_end_matches('0').to_string();
                }
                if !digits.is_empty() {
                    out.push(b[i] as char);
                    out.push_str(&digits);
                }
                i = j;
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(src: &str, data: serde_json::Value) -> String {
        render(src, &data, false, Utc::now()).unwrap()
    }

    #[test]
    fn go_template_basics() {
        let v = json!({"a": [1, 2, 3], "n": null, "o": {"k": "v"}, "f": 1.5});
        assert_eq!(r("{{range .a}}{{.}},{{end}}", v.clone()), "1,2,3,");
        assert_eq!(r("{{if .n}}y{{else}}n{{end}}", v.clone()), "n");
        assert_eq!(r("{{.o.k}} {{len .a}} {{.f}}", v.clone()), "v 3 1.5");
        assert_eq!(r(r#"{{printf "%05d" (index .a 2)}}"#, v), "00003");
    }

    #[test]
    fn tables_flush_at_end_and_on_tablerender() {
        let v = json!([{"a": "x", "b": 1}, {"a": "long", "b": null}]);
        assert_eq!(
            r("{{range .}}{{tablerow .a .b}}{{end}}", v.clone()),
            "x     1\nlong  \n"
        );
        assert_eq!(
            r(
                "{{range .}}{{tablerow .a}}{{end}}{{tablerender}}--{{tablerow \"z\"}}",
                v
            ),
            "x\nlong\n--z\n"
        );
    }

    #[test]
    fn scalars_like_gh() {
        assert_eq!(r("{{truncate 10 .}}", json!(2.0)), "2");
        assert_eq!(r("{{truncate 10 .}}", json!(2.345)), "2.35");
        assert_eq!(r("{{truncate 10 .}}", json!(null)), "");
        assert!(render("{{join \",\" .}}", &json!([{"a": 1}]), false, Utc::now()).is_err());
    }

    #[test]
    fn truncate_like_gh() {
        assert_eq!(truncate(5, "abcdef"), "ab...");
        assert_eq!(truncate(4, "abcdef"), "abcd");
        assert_eq!(truncate(6, "abcdef"), "abcdef");
        assert_eq!(truncate(6, "日本語日本語"), "日... ");
    }

    #[test]
    fn timeago_like_gh() {
        let now = DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ago = |s: &str| {
            time_ago(
                now,
                DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc),
            )
        };
        assert_eq!(ago("2026-03-01T11:59:30Z"), "less than a minute ago");
        assert_eq!(ago("2026-03-01T11:59:00Z"), "about 1 minute ago");
        assert_eq!(ago("2026-03-01T09:00:00Z"), "about 3 hours ago");
        assert_eq!(ago("2026-02-27T12:00:00Z"), "about 2 days ago");
        assert_eq!(ago("2025-12-01T12:00:00Z"), "about 3 months ago");
        assert_eq!(ago("2024-01-01T12:00:00Z"), "about 2 years ago");
        let out = render("{{timeago .}}", &json!("2026-03-01T11:00:00Z"), false, now).unwrap();
        assert_eq!(out, "about 1 hour ago");
    }

    #[test]
    fn timefmt_go_layouts() {
        let t = DateTime::parse_from_rfc3339("2026-01-05T15:04:05.120+02:00").unwrap();
        assert_eq!(
            go_time_format("2006-01-02T15:04:05Z07:00", &t),
            "2026-01-05T15:04:05+02:00"
        );
        assert_eq!(go_time_format("Mon Jan _2 3:04PM", &t), "Mon Jan  5 3:04PM");
        assert_eq!(
            go_time_format("January 2, 2006 .000 .999 -0700", &t),
            "January 5, 2026 .120 .12 +0200"
        );
        let z = DateTime::parse_from_rfc3339("2026-01-05T00:00:00Z").unwrap();
        assert_eq!(
            go_time_format("02/01/06 03pm MST Z07:00", &z),
            "05/01/26 12am UTC Z"
        );
    }

    #[test]
    fn helpers() {
        let v = json!({"ls": [{"name": "a"}, {"name": "b"}, {}]});
        assert_eq!(r(r#"{{join "+" (pluck "name" .ls)}}"#, v), "a+b+");
        assert_eq!(
            r(r#"{{color "red" "x"}}"#, json!(null)),
            "\x1b[0;31mx\x1b[0m"
        );
        assert_eq!(
            r(r#"{{color "green+bh:blue" "x"}}"#, json!(null)),
            "\x1b[0;1;92;44mx\x1b[0m"
        );
        assert_eq!(
            r(r#"{{color "208" "x"}}"#, json!(null)),
            "\x1b[0;38;5;208mx\x1b[0m"
        );
        assert_eq!(r(r#"{{autocolor "red" "x"}}"#, json!(null)), "x");
        let colored = render(r#"{{autocolor "red" "x"}}"#, &json!(null), true, Utc::now()).unwrap();
        assert_eq!(colored, "\x1b[0;31mx\x1b[0m");
        assert_eq!(
            r(r#"{{hyperlink "https://x" "t"}}"#, json!(null)),
            "\x1b]8;;https://x\x1b\\t\x1b]8;;\x1b\\"
        );
    }

    #[test]
    fn parse_errors_surface() {
        let e = render("{{.a", &json!({}), false, Utc::now()).unwrap_err();
        assert!(e.to_string().contains("template"), "{e}");
    }
}
