use clap::{Args, Subcommand};
use eyre::Result;
use gitea_api::types::ActionRunner;
use url::Url;

use crate::config::Config;
use crate::issues::atty_check;
use crate::json::{JsonArgs, write_json};
use crate::repo;

#[derive(Args, Default)]
struct ScopeArgs {
    /// Instance-wide (admin only)
    #[arg(long, conflicts_with = "org")]
    admin: bool,

    /// Organization name
    #[arg(short = 'o', long, value_name = "NAME")]
    org: Option<String>,
}

#[derive(Debug)]
enum Scope {
    Admin,
    Org(String),
    Repo { owner: String, name: String },
}

fn resolve_scope(
    scope: &ScopeArgs,
    repo_args: &repo::RepoArgs,
    config_url: &Url,
) -> Result<Scope> {
    if scope.admin {
        return Ok(Scope::Admin);
    }
    if let Some(org) = &scope.org {
        return Ok(Scope::Org(org.clone()));
    }
    let info = repo::resolve_repo(repo_args.repo.as_deref(), config_url).map_err(|e| {
        eyre::eyre!("{e}\nUse --admin, --org NAME, or -R OWNER/REPO to specify scope.")
    })?;
    Ok(Scope::Repo {
        owner: info.owner,
        name: info.name,
    })
}

#[derive(Args)]
pub struct RunnerCommand {
    #[command(flatten)]
    repo: repo::RepoArgs,

    #[command(subcommand)]
    action: RunnerAction,
}

#[derive(Subcommand)]
enum RunnerAction {
    /// List runners
    List(ListArgs),
    /// View a single runner
    View(ViewArgs),
    /// Delete a runner
    Delete(DeleteArgs),
    /// Print a runner registration token
    RegistrationToken(TokenArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    scope: ScopeArgs,

    #[command(flatten)]
    json: JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    /// Runner ID
    id: i64,

    /// Output raw JSON
    #[arg(long)]
    json: bool,

    #[command(flatten)]
    scope: ScopeArgs,
}

#[derive(Args)]
struct DeleteArgs {
    /// Runner ID
    id: i64,

    /// Skip confirmation prompt
    #[arg(short = 'y', long = "yes")]
    yes: bool,

    #[command(flatten)]
    scope: ScopeArgs,
}

#[derive(Args)]
struct TokenArgs {
    #[command(flatten)]
    scope: ScopeArgs,
}

impl RunnerCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            RunnerAction::List(args) => list_runners(&self.repo, args).await,
            RunnerAction::View(args) => view_runner(&self.repo, args).await,
            RunnerAction::Delete(args) => delete_runner(&self.repo, args).await,
            RunnerAction::RegistrationToken(args) => registration_token(&self.repo, args).await,
        }
    }
}

const RUNNER_FIELDS: &[&str] = &[
    "id",
    "name",
    "status",
    "busy",
    "disabled",
    "ephemeral",
    "labels",
];

async fn list_runners(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let scope = resolve_scope(&args.scope, repo_args, &config.url)?;

    let runners: Vec<ActionRunner> = match &scope {
        Scope::Admin => api
            .get_admin_runners()
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner()
            .runners,
        Scope::Org(o) => api
            .get_org_runners()
            .org(o.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner()
            .runners,
        Scope::Repo { owner, name } => api
            .get_repo_runners()
            .owner(owner.clone())
            .repo(name.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner()
            .runners,
    };

    if args.json.is_json() {
        return write_json(&args.json, &runners, RUNNER_FIELDS);
    }

    if runners.is_empty() {
        eprintln!("No runners found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<20} {:<8} {:<30} FLAGS",
            "ID", "NAME", "STATUS", "LABELS"
        );
    }

    for r in &runners {
        let id = r.id.unwrap_or(0);
        let name = truncate(r.name.as_deref().unwrap_or(""), 20);
        let status = r.status.as_deref().unwrap_or("");
        let labels = truncate(&labels_string(r), 30);
        let flags = flags_string(r);
        println!("{id:<6} {name:<20} {status:<8} {labels:<30} {flags}");
    }

    Ok(())
}

async fn view_runner(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let scope = resolve_scope(&args.scope, repo_args, &config.url)?;
    let id = args.id.to_string();

    let runner = match &scope {
        Scope::Admin => api
            .get_admin_runner()
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner(),
        Scope::Org(o) => api
            .get_org_runner()
            .org(o.clone())
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner(),
        Scope::Repo { owner, name } => api
            .get_repo_runner()
            .owner(owner.clone())
            .repo(name.clone())
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner(),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&runner)?);
        return Ok(());
    }

    let name = runner.name.as_deref().unwrap_or("(unnamed)");
    let id_n = runner.id.unwrap_or(0);
    println!("{name} (#{id_n})");
    println!("Status: {}", runner.status.as_deref().unwrap_or("unknown"));
    let labels = labels_string(&runner);
    if !labels.is_empty() {
        let pretty = labels.replace(',', ", ");
        println!("Labels: {pretty}");
    }
    let flags = flags_string(&runner);
    if !flags.is_empty() {
        println!("Flags: {flags}");
    }

    Ok(())
}

async fn delete_runner(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    use std::io::IsTerminal;

    let config = Config::load()?;
    let api = config.client()?;
    let scope = resolve_scope(&args.scope, repo_args, &config.url)?;
    let id = args.id.to_string();

    // 1. Non-TTY guard — fail fast before any API call.
    if !args.yes && !std::io::stdin().is_terminal() {
        eyre::bail!("refusing to delete without -y/--yes (stdin is not a TTY)");
    }

    // 2. Fetch runner to get its name (for the prompt). Skip if --yes was given,
    //    since we won't be prompting.
    if !args.yes {
        let runner = match &scope {
            Scope::Admin => api
                .get_admin_runner()
                .runner_id(id.clone())
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner(),
            Scope::Org(o) => api
                .get_org_runner()
                .org(o.clone())
                .runner_id(id.clone())
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner(),
            Scope::Repo { owner, name } => api
                .get_repo_runner()
                .owner(owner.clone())
                .repo(name.clone())
                .runner_id(id.clone())
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner(),
        };
        let runner_name = runner.name.as_deref().unwrap_or("(unnamed)");

        // 3. Confirm
        let prompt = format!("Delete runner #{} \"{}\"?", args.id, runner_name);
        let confirmed = inquire::Confirm::new(&prompt)
            .with_default(false)
            .prompt()?;
        if !confirmed {
            eprintln!("Cancelled");
            return Ok(());
        }
    }

    // 3. Delete
    match &scope {
        Scope::Admin => api
            .delete_admin_runner()
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?,
        Scope::Org(o) => api
            .delete_org_runner()
            .org(o.clone())
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?,
        Scope::Repo { owner, name: rname } => api
            .delete_repo_runner()
            .owner(owner.clone())
            .repo(rname.clone())
            .runner_id(id.clone())
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?,
    };

    eprintln!("Deleted runner #{}", args.id);
    Ok(())
}

async fn registration_token(repo_args: &repo::RepoArgs, args: &TokenArgs) -> Result<()> {
    use gitea_api::Method;

    let config = Config::load()?;
    let api = config.client()?;
    let scope = resolve_scope(&args.scope, repo_args, &config.url)?;

    let path = match &scope {
        Scope::Admin => "admin/actions/runners/registration-token".to_string(),
        Scope::Org(o) => format!("orgs/{o}/actions/runners/registration-token"),
        Scope::Repo { owner, name } => {
            format!("repos/{owner}/{name}/actions/runners/registration-token")
        }
    };

    let resp = api
        .raw_request(Method::POST, &path, None)
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        eyre::bail!("HTTP {status}: {body}");
    }

    let body: serde_json::Value = resp.json().await?;
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| eyre::eyre!("response did not contain a 'token' field: {body}"))?;
    println!("{token}");
    Ok(())
}

fn flags_string(r: &ActionRunner) -> String {
    let mut out = Vec::new();
    if r.busy.unwrap_or(false) {
        out.push("busy");
    }
    if r.disabled.unwrap_or(false) {
        out.push("disabled");
    }
    if r.ephemeral.unwrap_or(false) {
        out.push("ephemeral");
    }
    out.join(",")
}

fn labels_string(r: &ActionRunner) -> String {
    r.labels
        .iter()
        .filter_map(|l| l.name.as_deref())
        .collect::<Vec<_>>()
        .join(",")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(3);
        format!("{}...", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_url() -> Url {
        Url::parse("https://gitea.example.com").unwrap()
    }

    #[test]
    fn resolve_scope_admin_wins_over_org() {
        let scope = ScopeArgs {
            admin: true,
            org: Some("ignored".into()),
        };
        let repo = repo::RepoArgs::default();
        let r = resolve_scope(&scope, &repo, &fake_url()).unwrap();
        assert!(matches!(r, Scope::Admin));
    }

    #[test]
    fn resolve_scope_org_when_no_admin() {
        let scope = ScopeArgs {
            admin: false,
            org: Some("acme".into()),
        };
        let repo = repo::RepoArgs::default();
        let r = resolve_scope(&scope, &repo, &fake_url()).unwrap();
        assert!(matches!(r, Scope::Org(o) if o == "acme"));
    }

    #[test]
    fn resolve_scope_repo_explicit() {
        let scope = ScopeArgs {
            admin: false,
            org: None,
        };
        let repo = repo::RepoArgs {
            repo: Some("alice/proj".into()),
        };
        let r = resolve_scope(&scope, &repo, &fake_url()).unwrap();
        match r {
            Scope::Repo { owner, name } => {
                assert_eq!(owner, "alice");
                assert_eq!(name, "proj");
            }
            other => panic!("expected Repo, got {other:?}"),
        }
    }

    #[test]
    fn flags_string_empty_when_all_false() {
        let r = ActionRunner {
            busy: Some(false),
            disabled: Some(false),
            ephemeral: Some(false),
            id: Some(1),
            labels: vec![],
            name: Some("r".into()),
            status: Some("online".into()),
        };
        assert_eq!(flags_string(&r), "");
    }

    #[test]
    fn flags_string_lists_true_flags_in_order() {
        let r = ActionRunner {
            busy: Some(true),
            disabled: Some(false),
            ephemeral: Some(true),
            id: Some(1),
            labels: vec![],
            name: Some("r".into()),
            status: Some("online".into()),
        };
        assert_eq!(flags_string(&r), "busy,ephemeral");
    }

    #[test]
    fn labels_string_joins_with_comma() {
        use gitea_api::types::ActionRunnerLabel;
        let r = ActionRunner {
            busy: Some(false),
            disabled: Some(false),
            ephemeral: Some(false),
            id: Some(1),
            labels: vec![
                ActionRunnerLabel {
                    id: Some(1),
                    name: Some("self-hosted".into()),
                    type_: Some("custom".into()),
                },
                ActionRunnerLabel {
                    id: Some(2),
                    name: Some("linux".into()),
                    type_: Some("custom".into()),
                },
            ],
            name: Some("r".into()),
            status: Some("online".into()),
        };
        assert_eq!(labels_string(&r), "self-hosted,linux");
    }

    #[test]
    fn truncate_short_returns_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        assert_eq!(truncate("abcdefghij", 6), "abc...");
    }
}
