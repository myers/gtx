use clap::Args;
use eyre::Result;
use serde::Serialize;

/// Shared `--json` and `--jq` flags for list commands.
#[derive(Args, Clone, Default, Debug)]
pub struct JsonArgs {
    /// Output JSON with specified fields (comma-separated).
    /// With no fields, dumps the full object.
    /// Use `--json help` to list available fields.
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    pub json: Option<Vec<String>>,

    /// Filter JSON output with a jq expression (requires --json)
    #[arg(long = "jq", value_name = "EXPR", requires = "json")]
    pub jq_expr: Option<String>,
}

impl JsonArgs {
    /// Returns true if JSON output was requested.
    pub fn is_json(&self) -> bool {
        self.json.is_some()
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

/// Like [`write_json`], for a single object (e.g. a `view` command).
pub fn write_json_one<T: Serialize>(
    args: &JsonArgs,
    item: &T,
    available_fields: &[&str],
) -> Result<()> {
    write_json_value(args, serde_json::to_value(item)?, available_fields)
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
