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
    let fields = match &args.json {
        Some(f) if !f.is_empty() => f,
        _ => {
            // Bare --json with no fields: dump full objects
            return write_and_filter(args, &serde_json::to_value(items)?);
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

    // Serialize and filter to requested fields
    let full = serde_json::to_value(items)?;
    let filtered = filter_fields(&full, fields);
    write_and_filter(args, &filtered)
}

/// Filter a JSON array to only include specified fields on each object.
fn filter_fields(value: &serde_json::Value, fields: &[String]) -> serde_json::Value {
    match value {
        serde_json::Value::Array(arr) => {
            let filtered: Vec<serde_json::Value> = arr
                .iter()
                .map(|item| {
                    if let serde_json::Value::Object(obj) = item {
                        let mut filtered_obj = serde_json::Map::new();
                        for field in fields {
                            if let Some(v) = obj.get(field.as_str()) {
                                filtered_obj.insert(field.clone(), v.clone());
                            }
                        }
                        serde_json::Value::Object(filtered_obj)
                    } else {
                        item.clone()
                    }
                })
                .collect();
            serde_json::Value::Array(filtered)
        }
        _ => value.clone(),
    }
}

/// Pretty-print JSON, optionally applying jq filter.
fn write_and_filter(args: &JsonArgs, value: &serde_json::Value) -> Result<()> {
    if let Some(ref expr) = args.jq_expr {
        let results = jq_select(value, expr)?;
        for r in results {
            match r {
                serde_json::Value::String(s) => println!("{s}"),
                other => println!("{}", serde_json::to_string_pretty(&other)?),
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

/// Simple jq-like field selector.
/// Supports: .field, .[].field, .field.nested, .[].field.nested
pub fn jq_select(value: &serde_json::Value, expr: &str) -> Result<Vec<serde_json::Value>> {
    let expr = expr.trim_start_matches('.');
    if expr.is_empty() {
        return Ok(vec![value.clone()]);
    }

    let parts: Vec<&str> = expr.splitn(2, '.').collect();
    let (head, rest) = (parts[0], parts.get(1).copied());

    if head == "[]" {
        if let Some(arr) = value.as_array() {
            let mut results = Vec::new();
            for item in arr {
                if let Some(rest) = rest {
                    results.extend(jq_select(item, &format!(".{rest}"))?);
                } else {
                    results.push(item.clone());
                }
            }
            return Ok(results);
        }
        return Ok(vec![]);
    }

    if let Some(obj) = value.as_object() {
        if let Some(field_value) = obj.get(head) {
            if let Some(rest) = rest {
                return jq_select(field_value, &format!(".{rest}"));
            }
            return Ok(vec![field_value.clone()]);
        }
    }

    Ok(vec![])
}
