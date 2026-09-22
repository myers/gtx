use clap::Args;
use eyre::Result;

use crate::config::Config;
use crate::repo;

#[derive(Args)]
pub struct BrowseCommand {
    /// Issue or PR number to open, or empty for repo
    number: Option<i64>,

    #[command(flatten)]
    pub repo: repo::RepoArgs,

    /// Open repository settings
    #[arg(long)]
    settings: bool,
}

impl BrowseCommand {
    pub async fn run(&self) -> Result<()> {
        let config = Config::load()?;
        let repo_info = repo::resolve_repo(self.repo.repo.as_deref(), &config.url)?;

        let base = format!(
            "{}/{}/{}",
            config.url.as_str().trim_end_matches('/'),
            repo_info.owner,
            repo_info.name,
        );

        let url = if self.settings {
            format!("{base}/settings")
        } else if let Some(n) = self.number {
            format!("{base}/issues/{n}")
        } else {
            base
        };

        eprintln!("Opening {url}");
        open_url(&url)?;
        Ok(())
    }
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).status()?;
    }
    #[cfg(target_os = "linux")]
    {
        // Try xdg-open, then sensible-browser, then just print
        if std::process::Command::new("xdg-open")
            .arg(url)
            .status()
            .is_err()
        {
            if std::process::Command::new("sensible-browser")
                .arg(url)
                .status()
                .is_err()
            {
                println!("{url}");
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .status()?;
    }
    Ok(())
}
