use std::process::Command;

use eyre::Result;
use url::Url;

/// Detected repository info: owner and repo name
#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub name: String,
}

impl RepoInfo {
    #[allow(dead_code)]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Shared `-R`/`--repo` flag, added to parent commands with `global = true`.
#[derive(clap::Args, Clone, Default, Debug)]
pub struct RepoArgs {
    /// Repository (owner/repo). Detected from git remote if omitted.
    #[arg(short = 'R', long = "repo", global = true)]
    pub repo: Option<String>,
}

/// Parse an "owner/repo" string into RepoInfo
pub fn parse_repo(s: &str) -> Result<RepoInfo> {
    let parts: Vec<&str> = s.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        eyre::bail!("Repository must be in owner/repo format, got: {s}");
    }
    Ok(RepoInfo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

/// Resolve the repository: explicit `-R` flag > `GITEA_REPO` env > git remote detection.
/// `gitea_url` is the configured Gitea instance URL, used to match against remotes.
pub fn resolve_repo(explicit: Option<&str>, gitea_url: &Url) -> Result<RepoInfo> {
    // 1. Explicit -R flag
    if let Some(r) = explicit {
        return parse_repo(r);
    }

    // 2. GITEA_REPO env var
    if let Ok(r) = std::env::var("GITEA_REPO") {
        return parse_repo(&r);
    }

    // 3. Detect from git remote
    detect_repo_from_git(gitea_url)
}

/// Detect owner/repo from git remote URLs, matching against the Gitea instance URL.
fn detect_repo_from_git(gitea_url: &Url) -> Result<RepoInfo> {
    let output = Command::new("git")
        .args(["remote", "-v"])
        .output()
        .map_err(|_| eyre::eyre!("Failed to run git. Are you in a git repository?"))?;

    if !output.status.success() {
        eyre::bail!("Not a git repository (or git not found). Use -R owner/repo to specify.");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let gitea_host = gitea_url.host_str().unwrap_or("");

    // Try each remote, preferring "origin"
    let mut remotes: Vec<(&str, &str)> = Vec::new();
    for line in stdout.lines() {
        // Format: "origin\thttps://gitea.example.com/owner/repo.git (fetch)"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            remotes.push((parts[0], parts[1]));
        }
    }

    // Sort so "origin" comes first
    remotes.sort_by_key(|(name, _)| if *name == "origin" { 0 } else { 1 });

    for (_name, url_str) in &remotes {
        if let Some(info) = parse_remote_url(url_str, gitea_host) {
            return Ok(info);
        }
    }

    eyre::bail!(
        "No git remote matching Gitea instance {gitea_host} found. Use -R owner/repo to specify."
    )
}

/// Parse a git remote URL and extract owner/repo if it matches the Gitea host.
/// Supports:
///   - https://gitea.example.com/owner/repo.git
///   - https://gitea.example.com/owner/repo
///   - git@gitea.example.com:owner/repo.git
///   - ssh://git@gitea.example.com/owner/repo.git
///   - ssh://git@gitea.example.com:2222/owner/repo.git
fn parse_remote_url(url_str: &str, gitea_host: &str) -> Option<RepoInfo> {
    // Try SCP-style first: git@host:owner/repo.git
    if let Some(rest) = url_str
        .strip_prefix("git@")
        .or_else(|| url_str.strip_prefix("gitea@"))
    {
        let (host, path) = rest.split_once(':')?;
        if !host_matches(host, gitea_host) {
            return None;
        }
        return extract_owner_repo(path);
    }

    // Try as a URL (https://, ssh://, git://)
    if let Ok(parsed) = Url::parse(url_str) {
        let host = parsed.host_str()?;
        if !host_matches(host, gitea_host) {
            return None;
        }
        let path = parsed.path().trim_start_matches('/');
        return extract_owner_repo(path);
    }

    None
}

/// Extract owner/repo from a path like "owner/repo.git" or "owner/repo"
fn extract_owner_repo(path: &str) -> Option<RepoInfo> {
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(RepoInfo {
            owner: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    } else {
        None
    }
}

/// Check if a remote host matches the Gitea host (case-insensitive)
fn host_matches(remote_host: &str, gitea_host: &str) -> bool {
    remote_host.eq_ignore_ascii_case(gitea_host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo() {
        let r = parse_repo("owner/repo").unwrap();
        assert_eq!(r.owner, "owner");
        assert_eq!(r.name, "repo");

        assert!(parse_repo("noslash").is_err());
        assert!(parse_repo("/repo").is_err());
        assert!(parse_repo("owner/").is_err());
    }

    #[test]
    fn test_parse_remote_url_https() {
        let r = parse_remote_url("https://gitea.example.com/myorg/myrepo.git", "gitea.example.com")
            .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_parse_remote_url_https_no_dotgit() {
        let r =
            parse_remote_url("https://gitea.example.com/myorg/myrepo", "gitea.example.com")
                .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_parse_remote_url_ssh_scp() {
        let r =
            parse_remote_url("git@gitea.example.com:myorg/myrepo.git", "gitea.example.com")
                .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_parse_remote_url_ssh_url() {
        let r = parse_remote_url(
            "ssh://git@gitea.example.com/myorg/myrepo.git",
            "gitea.example.com",
        )
        .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_parse_remote_url_ssh_custom_port() {
        let r = parse_remote_url(
            "ssh://git@gitea.example.com:2222/myorg/myrepo.git",
            "gitea.example.com",
        )
        .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_parse_remote_url_wrong_host() {
        let r = parse_remote_url("https://github.com/myorg/myrepo.git", "gitea.example.com");
        assert!(r.is_none());
    }

    #[test]
    fn test_parse_remote_url_case_insensitive() {
        let r = parse_remote_url("https://Gitea.Example.COM/myorg/myrepo.git", "gitea.example.com")
            .unwrap();
        assert_eq!(r.owner, "myorg");
        assert_eq!(r.name, "myrepo");
    }

    #[test]
    fn test_slug() {
        let r = RepoInfo {
            owner: "alice".into(),
            name: "project".into(),
        };
        assert_eq!(r.slug(), "alice/project");
    }
}
