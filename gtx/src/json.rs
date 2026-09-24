use clap::Args;
use eyre::Result;
use serde::Serialize;

/// Shared `--json` and `--jq` flags.
#[derive(Args, Clone, Default, Debug)]
pub struct JsonArgs {
    /// Output JSON with the specified fields (comma-separated); with no
    /// fields, list the available ones
    #[arg(long, value_name = "FIELDS", value_delimiter = ',', num_args = 0..)]
    pub json: Option<Vec<String>>,

    /// Filter JSON output using a jq expression (requires --json)
    #[arg(short = 'q', long = "jq", value_name = "EXPR", requires = "json")]
    pub jq_expr: Option<String>,
}

impl JsonArgs {
    /// Returns true if JSON output was requested.
    pub fn is_json(&self) -> bool {
        self.json.is_some()
    }

    /// Check the requested `--json` fields against `fields`, the way gh does:
    /// bare `--json` lists the available fields and an unknown field is an
    /// error, both before anything is fetched. `Ok(None)` means no JSON was
    /// asked for.
    pub fn select<'f, T>(&self, fields: &'f [Field<T>]) -> Result<Option<Selection<'f, T>>> {
        let Some(requested) = &self.json else {
            return Ok(None);
        };
        let mut names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        names.sort_unstable();
        let list = names.iter().map(|n| format!("  {n}")).collect::<Vec<_>>().join("\n");
        if requested.is_empty() {
            eyre::bail!("Specify one or more comma-separated fields for `--json`:\n{list}");
        }
        let mut chosen = Vec::new();
        for name in requested {
            let Some(field) = fields.iter().find(|f| f.name == name) else {
                eyre::bail!("Unknown JSON field: {name:?}\nAvailable fields:\n{list}");
            };
            chosen.push(field);
        }
        Ok(Some(Selection {
            fields: chosen,
            jq: self.jq_expr.clone(),
        }))
    }
}

/// One gh-named `--json` field and how to compute it from a `T` (usually a
/// Gitea API type, or a wrapper carrying extra context such as "is latest").
pub struct Field<T> {
    pub name: &'static str,
    pub get: fn(&T) -> serde_json::Value,
}

/// Shorthand for building a [`Field`] table.
pub const fn field<T>(name: &'static str, get: fn(&T) -> serde_json::Value) -> Field<T> {
    Field { name, get }
}

/// The fields a user picked with `--json`, from [`JsonArgs::select`].
pub struct Selection<'f, T> {
    fields: Vec<&'f Field<T>>,
    jq: Option<String>,
}

impl<T> Selection<'_, T> {
    /// Whether `name` was requested, to skip fetching data nobody asked for.
    pub fn wants(&self, name: &str) -> bool {
        self.fields.iter().any(|f| f.name == name)
    }

    fn object(&self, item: &T) -> serde_json::Value {
        serde_json::Value::Object(
            self.fields
                .iter()
                .map(|f| (f.name.to_string(), (f.get)(item)))
                .collect(),
        )
    }

    /// Print `items` as a JSON array of the selected fields (or run `--jq`).
    pub fn write_list(&self, items: &[T]) -> Result<()> {
        let v = serde_json::Value::Array(items.iter().map(|i| self.object(i)).collect());
        self.write(&v)
    }

    /// Print one object of the selected fields (or run `--jq`).
    pub fn write_one(&self, item: &T) -> Result<()> {
        self.write(&self.object(item))
    }

    fn write(&self, value: &serde_json::Value) -> Result<()> {
        match &self.jq {
            Some(expr) => print_jq(value, expr),
            None => {
                println!("{}", serde_json::to_string_pretty(value)?);
                Ok(())
            }
        }
    }
}

/// Value helpers for [`Field`] getters: gh's shapes for common Gitea data.
pub mod gh {
    use gitea_api::types::{Label, Milestone, User};
    use serde_json::{Value, json};

    /// `Option<T>` (or anything serializable) as JSON, `null` when absent.
    pub fn v<T: serde::Serialize>(x: T) -> Value {
        serde_json::to_value(x).unwrap_or(Value::Null)
    }

    /// A timestamp the way gh prints one: RFC 3339, whole seconds, `Z`.
    pub fn time(t: Option<chrono::DateTime<chrono::Utc>>) -> Value {
        t.map_or(Value::Null, |t| {
            Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        })
    }

    /// gh's user object: `{id, login, name}`.
    pub fn user(u: Option<&User>) -> Value {
        u.map_or(Value::Null, |u| {
            json!({"id": u.id, "login": u.login, "name": u.full_name.as_deref().unwrap_or("")})
        })
    }

    pub fn users(us: &[User]) -> Value {
        Value::Array(us.iter().map(|u| user(Some(u))).collect())
    }

    /// gh's label object: `{id, name, description, color}`.
    pub fn labels(ls: &[Label]) -> Value {
        Value::Array(
            ls.iter()
                .map(|l| json!({"id": l.id, "name": l.name, "description": l.description.as_deref().unwrap_or(""), "color": l.color}))
                .collect(),
        )
    }

    /// gh's milestone object: `{number, title, description, dueOn}`.
    pub fn milestone(m: Option<&Milestone>) -> Value {
        m.map_or(Value::Null, |m| {
            json!({"number": m.id, "title": m.title, "description": m.description.as_deref().unwrap_or(""), "dueOn": time(m.due_on)})
        })
    }
}

/// Write items as JSON, optionally filtering to specific fields and applying jq.
///
/// `available_fields` is the whitelist of valid field names for this command.
/// If the user passes `--json help`, prints the available fields and returns Ok.
/// If no field names given (bare `--json`), dumps the full object.
pub fn write_json<T: Serialize>(
    args: &JsonArgs,
    items: &[T],
    available_fields: &[&str],
) -> Result<()> {
    write_json_value(args, serde_json::to_value(items)?, available_fields)
}

fn write_json_value(
    args: &JsonArgs,
    full: serde_json::Value,
    available_fields: &[&str],
) -> Result<()> {
    let fields = match &args.json {
        Some(f) if !f.is_empty() => f,
        _ => {
            // Bare --json with no fields: dump full objects
            return write_and_filter(args, &full);
        }
    };

    // --json help: list available fields
    if fields.len() == 1 && fields[0] == "help" {
        eprintln!("Available JSON fields:");
        for f in available_fields {
            eprintln!("  {f}");
        }
        return Ok(());
    }

    // Validate requested fields
    for f in fields {
        if !available_fields.contains(&f.as_str()) {
            eyre::bail!(
                "Unknown field: {f}\nAvailable fields: {}",
                available_fields.join(", ")
            );
        }
    }

    let filtered = filter_fields(&full, fields);
    write_and_filter(args, &filtered)
}

/// Keep only `fields` of an object, or of each object in an array.
fn filter_fields(value: &serde_json::Value, fields: &[String]) -> serde_json::Value {
    match value {
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|item| filter_fields(item, fields)).collect())
        }
        serde_json::Value::Object(obj) => serde_json::Value::Object(
            fields
                .iter()
                .filter_map(|f| Some((f.clone(), obj.get(f.as_str())?.clone())))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Pretty-print JSON, optionally applying jq filter.
fn write_and_filter(args: &JsonArgs, value: &serde_json::Value) -> Result<()> {
    if let Some(ref expr) = args.jq_expr {
        print_jq(value, expr)
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }
}

/// Run a jq filter over `value` and print each result on its own line.
pub fn print_jq(value: &serde_json::Value, expr: &str) -> Result<()> {
    for line in jq_lines(value, expr)? {
        println!("{line}");
    }
    Ok(())
}

/// Run a jq filter over `value`, returning one line per output value, the
/// way `gh --jq` does: strings raw, everything else as compact JSON.
pub fn jq_lines(value: &serde_json::Value, expr: &str) -> Result<Vec<String>> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
    use jaq_json::Val;

    let input = jaq_json::read::parse_single(serde_json::to_string(value)?.as_bytes())
        .map_err(|e| eyre::eyre!("jq input: {e}"))?;

    let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
    let funs = jaq_core::funs().chain(jaq_std::funs()).chain(jaq_json::funs());
    let arena = Arena::default();
    let modules = Loader::new(defs)
        .load(&arena, File { code: expr, path: () })
        .map_err(|_| eyre::eyre!("invalid jq expression: {expr}"))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|_| eyre::eyre!("invalid jq expression: {expr}"))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input))
        .map(unwrap_valr)
        .map(|r| match r {
            Ok(Val::TStr(s)) => Ok(String::from_utf8_lossy(&s).into_owned()),
            Ok(v) => Ok(v.to_string()),
            Err(e) => Err(eyre::eyre!("jq: {e}")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jq_multi_output_prints_each_result() {
        let v = json!({"full_name": "chaos-inc/drawbar-smoke", "private": true});
        assert_eq!(
            jq_lines(&v, ".full_name,.private").unwrap(),
            vec!["chaos-inc/drawbar-smoke", "true"]
        );
    }

    #[test]
    fn jq_array_index() {
        let v = json!([{"id": 252}, {"id": 251}]);
        assert_eq!(jq_lines(&v, ".[0].id").unwrap(), vec!["252"]);
    }

    #[test]
    fn jq_iterate_field() {
        let v = json!([{"name": "a"}, {"name": "b"}]);
        assert_eq!(jq_lines(&v, ".[].name").unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn jq_non_strings_are_compact_json() {
        let v = json!({"o": {"a": [1, 2]}});
        assert_eq!(jq_lines(&v, ".o").unwrap(), vec![r#"{"a":[1,2]}"#]);
    }

    #[test]
    fn jq_invalid_expression_is_an_error() {
        assert!(jq_lines(&json!({}), ".foo[").is_err());
    }

    #[test]
    fn jq_runtime_error_is_an_error() {
        assert!(jq_lines(&json!({"a": 1}), ".a[0]").is_err());
    }
}
