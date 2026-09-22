use clap::{Args, Subcommand};
use eyre::Result;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

#[derive(Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    action: AuthAction,
}

#[derive(Subcommand)]
enum AuthAction {
    /// Log in to a Gitea instance
    Login(LoginArgs),
    /// Show current authentication status
    Status(AuthStatusArgs),
    /// Log out (remove config file)
    Logout,
    /// Configure git to use gt as credential helper
    SetupGit(SetupGitArgs),
    /// Git credential helper (used by git, not invoked directly)
    GitCredential(GitCredentialArgs),
}

#[derive(Args)]
struct AuthStatusArgs {
    /// Skip the network probe and report only the locally stored config.
    /// Useful for shell prompts that need zero-network output.
    #[arg(long = "no-check")]
    no_check: bool,
}

#[derive(Args)]
struct LoginArgs {
    /// Gitea instance URL
    #[arg(long)]
    url: Option<String>,

    /// API token
    #[arg(long)]
    token: Option<String>,
}

#[derive(Args)]
struct SetupGitArgs {
    /// The hostname to configure git for (e.g., gitea.example.com)
    #[arg(short = 'H', long)]
    hostname: Option<String>,

    /// Force setup even if the host is not authenticated. Requires --hostname.
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct GitCredentialArgs {
    /// Operation: get, store, or erase
    operation: String,
}

impl AuthCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            AuthAction::Login(args) => login(args),
            AuthAction::Status(args) => status(args).await,
            AuthAction::Logout => logout(),
            AuthAction::SetupGit(args) => setup_git(args),
            AuthAction::GitCredential(args) => git_credential(args),
        }
    }
}

fn config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "gt")
        .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn login(args: &LoginArgs) -> Result<()> {
    let url = match &args.url {
        Some(u) => u.clone(),
        None => {
            eprint!("Gitea URL: ");
            io::stderr().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    let token = match &args.token {
        Some(t) => t.clone(),
        None => {
            eprint!("API token: ");
            io::stderr().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if url.is_empty() || token.is_empty() {
        eyre::bail!("URL and token are required");
    }

    // Validate URL
    url::Url::parse(&url).map_err(|e| eyre::eyre!("Invalid URL: {e}"))?;

    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        "[default]\nurl = \"{url}\"\ntoken = \"{token}\"\n"
    );
    std::fs::write(&path, content)?;

    eprintln!("Logged in to {url}");
    eprintln!("Config saved to {}", path.display());
    eprintln!("hint: run `gt auth setup-git` to configure git authentication");
    Ok(())
}

async fn status(args: &AuthStatusArgs) -> Result<()> {
    // Resolve config the same way every other gt command does, so the
    // displayed url/token match what a probe would actually use. This means
    // env vars (GITEA_URL, GITEA_TOKEN) win over the file — and the user
    // sees that in the output.
    let api_config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            let path = config_path()?;
            if !path.exists() {
                eprintln!("Not logged in (no config file at {})", path.display());
            } else {
                eprintln!("{e}");
            }
            eyre::bail!("not logged in");
        }
    };

    let path = config_path()?;
    let path_display = if path.exists() {
        path.display().to_string()
    } else {
        "(env vars only)".to_string()
    };

    println!("URL:   {}", api_config.url);
    println!("Token: {}", mask_token(&api_config.token));
    println!("Config: {path_display}");

    if args.no_check {
        return Ok(());
    }

    let api = api_config
        .client()
        .map_err(|e| eyre::eyre!("could not build API client: {e}"))?;

    match api.user_get_current().send().await {
        Ok(rv) => {
            let user = rv.into_inner();
            let login = user.login.as_deref().unwrap_or("(unknown)");
            let admin_tag = if user.is_admin.unwrap_or(false) {
                " (admin)"
            } else {
                ""
            };
            println!("Logged in as {login}{admin_tag} — token valid");
            Ok(())
        }
        Err(e) => {
            let status = e.status().map(|s| s.as_u16());
            match status {
                Some(401) => {
                    eprintln!(
                        "Token rejected by server. Run `gt auth login` to refresh."
                    );
                    eyre::bail!("token invalid (401)");
                }
                Some(code) => {
                    eprintln!("Probe failed: server returned {code}");
                    eyre::bail!("probe failed (HTTP {code})");
                }
                None => {
                    eprintln!("Probe failed: {e}");
                    eyre::bail!("probe failed");
                }
            }
        }
    }
}

fn mask_token(token: &str) -> String {
    if token.len() > 8 {
        format!("{}...{}", &token[..4], &token[token.len() - 4..])
    } else if !token.is_empty() {
        "****".to_string()
    } else {
        "(not set)".to_string()
    }
}

fn logout() -> Result<()> {
    let path = config_path()?;

    if !path.exists() {
        eprintln!("Already logged out (no config file)");
        return Ok(());
    }

    std::fs::remove_file(&path)?;
    eprintln!("Logged out (removed {})", path.display());
    Ok(())
}

fn setup_git(args: &SetupGitArgs) -> Result<()> {
    if args.force && args.hostname.is_none() {
        eyre::bail!("--force requires --hostname");
    }

    let hosts = if let Some(ref hostname) = args.hostname {
        let scheme = if hostname.starts_with("http://") || hostname.starts_with("https://") {
            let parsed = url::Url::parse(hostname)
                .map_err(|e| eyre::eyre!("Invalid URL: {e}"))?;
            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(hostname))
        } else {
            format!("https://{hostname}")
        };

        if !args.force {
            crate::config::Config::load().map_err(|_| {
                eyre::eyre!(
                    "Host is not authenticated. Use --force to set up anyway, or run `gt auth login` first."
                )
            })?;
        }

        vec![scheme]
    } else {
        let config = crate::config::Config::load()?;
        let host = format!(
            "{}://{}",
            config.url.scheme(),
            config.url.host_str().ok_or_else(|| eyre::eyre!("No host in configured URL"))?,
        );
        vec![host]
    };

    let gt_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "gt".to_string());

    let helper_value = format!("!{gt_path} auth git-credential");

    for host in &hosts {
        let key = format!("credential.{host}.helper");

        let existing = std::process::Command::new("git")
            .args(["config", "--global", "--get-all", &key])
            .output()?;

        let already_set = String::from_utf8_lossy(&existing.stdout)
            .lines()
            .any(|line| line.trim() == helper_value);

        if !already_set {
            let has_reset = String::from_utf8_lossy(&existing.stdout)
                .lines()
                .any(|line| line.trim().is_empty());

            if !has_reset {
                std::process::Command::new("git")
                    .args(["config", "--global", "--add", &key, ""])
                    .status()?;
            }

            let status = std::process::Command::new("git")
                .args(["config", "--global", "--add", &key, &helper_value])
                .status()?;
            if !status.success() {
                eyre::bail!("Failed to configure git credential helper for {host}");
            }
        }

        eprintln!("Configured git credential helper for {host}");
        eprintln!("  {key}={helper_value}");
    }

    Ok(())
}

fn git_credential(args: &GitCredentialArgs) -> Result<()> {
    if args.operation != "get" {
        return Ok(());
    }

    let mut protocol = String::new();
    let mut host = String::new();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
        if let Some(val) = line.strip_prefix("protocol=") {
            protocol = val.to_string();
        } else if let Some(val) = line.strip_prefix("host=") {
            host = val.to_string();
        }
    }

    let config = crate::config::Config::load()?;

    let config_host = config.url.host_str().unwrap_or("");
    let config_scheme = config.url.scheme();

    if host == config_host && protocol == config_scheme {
        println!("protocol={protocol}");
        println!("host={host}");
        println!("username=token");
        println!("password={}", config.token);
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::mask_token;

    #[test]
    fn mask_long_token() {
        assert_eq!(mask_token("aafe123456789a263"), "aafe...a263");
    }

    #[test]
    fn mask_short_token() {
        assert_eq!(mask_token("short"), "****");
    }

    #[test]
    fn mask_empty_token() {
        assert_eq!(mask_token(""), "(not set)");
    }
}

