use clap::{Args, Subcommand};
use eyre::Result;
use std::path::PathBuf;

#[derive(Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Get a config value
    Get(GetArgs),
    /// Set a config value
    Set(SetArgs),
    /// List all config values
    List,
}

#[derive(Args)]
struct GetArgs {
    /// Config key (e.g., "default.url")
    key: String,
}

#[derive(Args)]
struct SetArgs {
    /// Config key (e.g., "default.url")
    key: String,

    /// Value to set
    value: String,
}

impl ConfigCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            ConfigAction::Get(args) => get_config(args),
            ConfigAction::Set(args) => set_config(args),
            ConfigAction::List => list_config(),
        }
    }
}

fn config_path() -> Result<PathBuf> {
    crate::config::config_file()
}

fn load_config() -> Result<toml::Value> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let content = std::fs::read_to_string(&path)?;
    let config: toml::Value = toml::from_str(&content)?;
    Ok(config)
}

fn get_config(args: &GetArgs) -> Result<()> {
    let config = load_config()?;
    let parts: Vec<&str> = args.key.split('.').collect();

    let mut current = &config;
    for part in &parts {
        current = current
            .get(part)
            .ok_or_else(|| eyre::eyre!("Key '{}' not found", args.key))?;
    }

    match current {
        toml::Value::String(s) => println!("{s}"),
        other => println!("{other}"),
    }
    Ok(())
}

fn set_config(args: &SetArgs) -> Result<()> {
    let parts: Vec<&str> = args.key.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        eyre::bail!("Invalid config key '{}'", args.key);
    }
    let (key, tables) = parts.split_last().expect("split yields at least one part");

    crate::config::edit_config_file(|doc| {
        let value = toml_edit::value(args.value.as_str());
        crate::config::table_mut(doc, tables)?.insert(key, value);
        Ok(())
    })?;
    eprintln!("Set {} = {}", args.key, args.value);
    Ok(())
}

fn list_config() -> Result<()> {
    let config = load_config()?;
    let mut lines = Vec::new();
    if let toml::Value::Table(table) = &config {
        flatten("", table, &mut lines);
    }
    if lines.is_empty() {
        eprintln!("No config values set");
    }
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// Flatten nested tables into `a.b.c = value` lines. Tokens are masked the
/// way `auth status` shows them; `gtx auth token` prints the real one.
fn flatten(prefix: &str, table: &toml::Table, out: &mut Vec<String>) {
    for (key, value) in table {
        let full = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            toml::Value::Table(inner) => flatten(&full, inner, out),
            toml::Value::String(s) if key == "token" => {
                out.push(format!("{full} = {}", crate::auth::mask_token(s)))
            }
            toml::Value::String(s) => out.push(format!("{full} = {s}")),
            other => out.push(format!("{full} = {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path() {
        let path = config_path().unwrap();
        assert!(path.to_str().unwrap().contains("gtx"));
        assert!(path.to_str().unwrap().ends_with("config.toml"));
    }
}
