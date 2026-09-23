use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
                    "No Gitea URL configured. Set GITEA_URL or add url to ~/.config/gtx/config.toml"
                )
            })?;

        let token = std::env::var("GITEA_TOKEN")
            .ok()
            .or(profile.token)
            .ok_or_else(|| {
                eyre::eyre!(
                    "No Gitea token configured. Set GITEA_TOKEN or add token to ~/.config/gtx/config.toml"
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
    let path = config_file().ok()?;
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Path of the config file: `GTX_CONFIG` if set, else `config.toml` in the
/// platform config dir (`~/.config/gtx` on Linux). The file may not exist yet.
pub fn config_file() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GTX_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let dirs = directories::ProjectDirs::from("", "", "gtx")
        .ok_or_else(|| eyre::eyre!("Cannot determine config directory"))?;
    if let Some(old) = directories::ProjectDirs::from("", "", "gt") {
        migrate_config_dir(old.config_dir(), dirs.config_dir())?;
    }
    Ok(dirs.config_dir().join("config.toml"))
}

/// One-time migration from the CLI's old name: when `new` doesn't exist yet
/// and `old` holds a config file, copy it across. `old` is left in place.
fn migrate_config_dir(old: &Path, new: &Path) -> Result<()> {
    let old_file = old.join("config.toml");
    if new.exists() || !old_file.is_file() {
        return Ok(());
    }
    let content = std::fs::read(&old_file)
        .wrap_err_with(|| format!("Migrating {} to {}", old.display(), new.display()))?;
    write_config_file(&new.join("config.toml"), content)
        .wrap_err_with(|| format!("Migrating {} to {}", old.display(), new.display()))?;
    eprintln!("Migrated config from {} to {}", old.display(), new.display());
    Ok(())
}

/// Write the config file with owner-only permissions (0600 on Unix), since it
/// holds API tokens. Creates parent dirs, and tightens the mode of an existing
/// file too (the mode passed at open only applies on creation).
pub fn write_config_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    use std::io::Write;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        let file = opts.open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        (&file).write_all(contents.as_ref())?;
    }
    #[cfg(not(unix))]
    opts.open(path)?.write_all(contents.as_ref())?;
    Ok(())
}

/// Load aliases from config file. Returns empty map if no config or no aliases.
pub fn load_aliases() -> HashMap<String, String> {
    load_config_file()
        .map(|c| c.aliases)
        .unwrap_or_default()
}

/// Ensure config directory and file exist, returning the path.
fn ensure_config_file() -> Result<PathBuf> {
    let path = config_file()?;
    if !path.exists() {
        write_config_file(&path, "")?;
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

    write_config_file(&path, doc.to_string())?;
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
        write_config_file(&path, doc.to_string())?;
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

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn test_write_config_file_creates_private_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub").join("config.toml");

        write_config_file(&path, "token = \"t\"\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "token = \"t\"\n");
        assert_eq!(mode(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_config_file_tightens_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "old contents that are longer").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_config_file(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(mode(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_migrate_config_dir_writes_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("gt");
        let new = tmp.path().join("gtx");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "x").unwrap();
        std::fs::set_permissions(old.join("config.toml"), std::fs::Permissions::from_mode(0o644))
            .unwrap();

        migrate_config_dir(&old, &new).unwrap();

        assert_eq!(mode(&new.join("config.toml")), 0o600);
    }

    #[test]
    fn test_migrate_config_dir_copies_old_file() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("gt");
        let new = tmp.path().join("gtx");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "[default]\nurl = \"u\"\n").unwrap();

        migrate_config_dir(&old, &new).unwrap();

        assert_eq!(
            std::fs::read_to_string(new.join("config.toml")).unwrap(),
            "[default]\nurl = \"u\"\n"
        );
        assert!(old.join("config.toml").exists());
    }

    #[test]
    fn test_migrate_config_dir_runs_once() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("gt");
        let new = tmp.path().join("gtx");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("config.toml"), "old").unwrap();

        migrate_config_dir(&old, &new).unwrap();

        assert!(!new.join("config.toml").exists());
    }

    #[test]
    fn test_migrate_config_dir_without_old_config() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("gtx");

        migrate_config_dir(&tmp.path().join("gt"), &new).unwrap();

        assert!(!new.exists());
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
