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
    let value = toml_edit::value(args.value.as_str());

    crate::config::edit_config_file(|doc| {
        match parts[..] {
            [section, key] => {
                crate::config::section_mut(doc, section)?.insert(key, value);
            }
            [key] => {
                doc.insert(key, value);
            }
            _ => eyre::bail!("Key must be 'key' or 'section.key' format"),
        }
        Ok(())
    })?;
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
        assert!(path.to_str().unwrap().contains("gtx"));
        assert!(path.to_str().unwrap().ends_with("config.toml"));
    }
}
