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
    let dirs = directories::ProjectDirs::from("", "", "gt")
        .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
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

fn save_config(config: &toml::Value) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
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
    let mut config = load_config()?;
    let parts: Vec<&str> = args.key.split('.').collect();

    if parts.len() == 2 {
        let table = config
            .as_table_mut()
            .ok_or_else(|| eyre::eyre!("Config is not a table"))?;
        let section = table
            .entry(parts[0])
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        let section_table = section
            .as_table_mut()
            .ok_or_else(|| eyre::eyre!("Section '{}' is not a table", parts[0]))?;
        section_table.insert(
            parts[1].to_string(),
            toml::Value::String(args.value.clone()),
        );
    } else if parts.len() == 1 {
        let table = config
            .as_table_mut()
            .ok_or_else(|| eyre::eyre!("Config is not a table"))?;
        table.insert(
            parts[0].to_string(),
            toml::Value::String(args.value.clone()),
        );
    } else {
        eyre::bail!("Key must be 'key' or 'section.key' format");
    }

    save_config(&config)?;
    eprintln!("Set {} = {}", args.key, args.value);
    Ok(())
}

fn list_config() -> Result<()> {
    let config = load_config()?;

    if let Some(table) = config.as_table() {
        if table.is_empty() {
            eprintln!("No config values set");
            return Ok(());
        }
        for (section, value) in table {
            if let Some(inner) = value.as_table() {
                for (key, val) in inner {
                    match val {
                        toml::Value::String(s) => println!("{section}.{key} = {s}"),
                        other => println!("{section}.{key} = {other}"),
                    }
                }
            } else {
                match value {
                    toml::Value::String(s) => println!("{section} = {s}"),
                    other => println!("{section} = {other}"),
                }
            }
        }
    } else {
        eprintln!("No config values set");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path() {
        let path = config_path().unwrap();
        assert!(path.to_str().unwrap().contains("gt"));
        assert!(path.to_str().unwrap().ends_with("config.toml"));
    }
}
