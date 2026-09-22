use std::collections::HashMap;
use std::path::PathBuf;

use eyre::{Result, WrapErr};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    default: ConfigProfile,
    #[serde(default)]
    servers: HashMap<String, ConfigProfile>,
    #[serde(default)]
    aliases: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ConfigProfile {
    url: Option<String>,
    token: Option<String>,
}

#[derive(Debug)]
pub struct Config {
    pub url: url::Url,
    pub token: String,
}

impl Config {
    /// Load config from file and env vars.
    ///
    /// Resolution order for server selection:
    /// 1. `GITEA_SERVER` env var selects a named server from `[servers.NAME]`
    /// 2. Falls back to `[default]` section
    ///
    /// Within the selected profile, env vars override file values:
    /// - `GITEA_URL` overrides the profile's url
    /// - `GITEA_TOKEN` overrides the profile's token
    pub fn load() -> Result<Self> {
        let file_config = load_config_file().unwrap_or_default();

        // Select the profile: named server or default
        let profile = if let Ok(server_name) = std::env::var("GITEA_SERVER") {
            file_config
                .servers
                .get(&server_name)
                .cloned()
                .ok_or_else(|| {
                    let available: Vec<&String> = file_config.servers.keys().collect();
                    eyre::eyre!(
                        "Server '{}' not found in config. Available: {:?}",
                        server_name,
                        available
                    )
                })?
        } else {
            file_config.default
        };

        let url_str = std::env::var("GITEA_URL")
            .ok()
            .or(profile.url)
            .ok_or_else(|| {
                eyre::eyre!(
                    "No Gitea URL configured. Set GITEA_URL or add url to ~/.config/gt/config.toml"
                )
            })?;

        let token = std::env::var("GITEA_TOKEN")
            .ok()
            .or(profile.token)
            .ok_or_else(|| {
                eyre::eyre!(
                    "No Gitea token configured. Set GITEA_TOKEN or add token to ~/.config/gt/config.toml"
                )
            })?;

        let url =
            url::Url::parse(&url_str).wrap_err_with(|| format!("Invalid URL: {url_str}"))?;

        Ok(Config { url, token })
    }

    /// Create a Gitea API client from this config.
    pub fn client(&self) -> Result<gitea_api::Gitea> {
        gitea_api::Gitea::new(gitea_api::Auth::Token(&self.token), self.url.clone())
            .map_err(|e| eyre::eyre!("{e}"))
    }
}

fn parse_config_toml(content: &str) -> Option<ConfigFile> {
    toml::from_str(content).ok()
}

fn load_config_file() -> Option<ConfigFile> {
    let path = config_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    parse_config_toml(&content)
}

pub fn config_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "gt")?;
    let path = dirs.config_dir().join("config.toml");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}


/// Load aliases from config file. Returns empty map if no config or no aliases.
pub fn load_aliases() -> HashMap<String, String> {
    load_config_file()
        .map(|c| c.aliases)
        .unwrap_or_default()
}

/// Ensure config directory and file exist, returning the path.
fn ensure_config_file() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "gt")
        .ok_or_else(|| eyre::eyre!("Cannot determine config directory"))?;
    let dir = dirs.config_dir();
    std::fs::create_dir_all(dir)?;
    let path = dir.join("config.toml");
    if !path.exists() {
        std::fs::write(&path, "")?;
    }
    Ok(path)
}

/// Set an alias in the config file.
pub fn set_alias(name: &str, expansion: &str) -> Result<()> {
    let path = ensure_config_file()?;
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml::Table = content.parse().unwrap_or_default();

    let aliases = doc
        .entry("aliases")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(t) = aliases {
        t.insert(name.to_string(), toml::Value::String(expansion.to_string()));
    }

    std::fs::write(&path, doc.to_string())?;
    Ok(())
}

/// Delete an alias from the config file.
pub fn delete_alias(name: &str) -> Result<bool> {
    let path = ensure_config_file()?;
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml::Table = content.parse().unwrap_or_default();

    let removed = if let Some(toml::Value::Table(t)) = doc.get_mut("aliases") {
        t.remove(name).is_some()
    } else {
        false
    };

    if removed {
        std::fs::write(&path, doc.to_string())?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_toml_full() {
        let toml = r#"
[default]
url = "https://gitea.example.com"
token = "abc123"
"#;
        let config = parse_config_toml(toml).unwrap();
        assert_eq!(
            config.default.url.as_deref(),
            Some("https://gitea.example.com")
        );
        assert_eq!(config.default.token.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_parse_config_toml_empty() {
        let config = parse_config_toml("").unwrap();
        assert!(config.default.url.is_none());
        assert!(config.default.token.is_none());
    }

    #[test]
    fn test_parse_config_toml_partial() {
        let toml = r#"
[default]
url = "https://gitea.example.com"
"#;
        let config = parse_config_toml(toml).unwrap();
        assert_eq!(
            config.default.url.as_deref(),
            Some("https://gitea.example.com")
        );
        assert!(config.default.token.is_none());
    }

    #[test]
    fn test_parse_config_toml_invalid() {
        let result = parse_config_toml("not valid toml {{{{");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_config_toml_multi_instance() {
        let toml = r#"
[default]
url = "https://gitea.example.com"
token = "default-token"

[servers.work]
url = "https://gitea.work.com"
token = "work-token"

[servers.personal]
url = "https://my.gitea.org"
token = "personal-token"
"#;
        let config = parse_config_toml(toml).unwrap();
        assert_eq!(
            config.default.url.as_deref(),
            Some("https://gitea.example.com")
        );
        assert_eq!(config.servers.len(), 2);
        assert_eq!(
            config.servers["work"].url.as_deref(),
            Some("https://gitea.work.com")
        );
        assert_eq!(
            config.servers["work"].token.as_deref(),
            Some("work-token")
        );
        assert_eq!(
            config.servers["personal"].url.as_deref(),
            Some("https://my.gitea.org")
        );
    }
}
