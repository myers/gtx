use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct LabelCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: LabelAction,
}

#[derive(Subcommand)]
enum LabelAction {
    /// List labels
    List(ListArgs),
    /// Create a label
    Create(CreateArgs),
    /// Edit a label
    Edit(EditArgs),
    /// Delete a label
    Delete(DeleteArgs),
    /// Clone labels from another repository
    Clone(CloneArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// Label name
    #[arg(short, long)]
    name: String,

    /// Label color (hex, e.g. "ff0000")
    #[arg(short, long)]
    color: String,

    /// Label description
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct EditArgs {
    /// Label ID
    id: i64,

    /// New name
    #[arg(short, long)]
    name: Option<String>,

    /// New color (hex)
    #[arg(short, long)]
    color: Option<String>,

    /// New description
    #[arg(short, long)]
    description: Option<String>,
}

#[derive(Args)]
struct DeleteArgs {
    /// Label ID
    id: i64,
}

#[derive(Args)]
struct CloneArgs {
    /// Source repository (owner/repo) to copy labels from
    from: String,
}

impl LabelCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            LabelAction::List(args) => list_labels(&self.repo, args).await,
            LabelAction::Create(args) => create_label(&self.repo, args).await,
            LabelAction::Edit(args) => edit_label(&self.repo, args).await,
            LabelAction::Delete(args) => delete_label(&self.repo, args).await,
            LabelAction::Clone(args) => clone_labels(&self.repo, args).await,
        }
    }
}

const LABEL_FIELDS: &[&str] = &["id", "name", "color", "description", "url"];

async fn list_labels(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let labels = paginate::paginate(200, 50, |page, per_page| {
        let api = &api;
        async move {
            Ok(api
                .issue_list_labels()
                .owner(owner)
                .repo(repo)
                .page(page)
                .limit(per_page)
                .send()
                .await
                .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
                .into_inner())
        }
    })
    .await?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &labels, &LABEL_FIELDS);
    }

    if labels.is_empty() {
        eprintln!("No labels found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<30} {:<10} {}", "ID", "NAME", "COLOR", "DESCRIPTION");
    }

    for label in &labels {
        let id = label.id.unwrap_or(0);
        let name = label.name.as_deref().unwrap_or("");
        let color = label.color.as_deref().unwrap_or("");
        let desc = label.description.as_deref().unwrap_or("");
        println!("{:<6} {:<30} {:<10} {}", id, name, color, desc);
    }

    Ok(())
}

async fn create_label(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let description = args.description.clone();
    let label = api
        .issue_create_label()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            b = b.name(args.name.clone()).color(args.color.clone());
            if let Some(desc) = description {
                b = b.description(desc);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = label.id.unwrap_or(0);
    let name = label.name.as_deref().unwrap_or("");
    eprintln!("Created label #{id}: {name}");
    Ok(())
}

async fn edit_label(repo_args: &repo::RepoArgs, args: &EditArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let name = args.name.clone();
    let color = args.color.clone();
    let description = args.description.clone();

    api.issue_edit_label()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .body_map(move |mut b| {
            if let Some(n) = name {
                b = b.name(n);
            }
            if let Some(c) = color {
                b = b.color(c);
            }
            if let Some(d) = description {
                b = b.description(d);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Label #{} updated", args.id);
    Ok(())
}

async fn delete_label(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.issue_delete_label()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Label #{} deleted", args.id);
    Ok(())
}

async fn clone_labels(repo_args: &repo::RepoArgs, args: &CloneArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    // Parse source repo
    let source = repo::parse_repo(&args.from)?;

    // Resolve target repo
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (target_owner, target_repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Fetch labels from source
    let source_labels = api
        .issue_list_labels()
        .owner(&source.owner)
        .repo(&source.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if source_labels.is_empty() {
        eprintln!("No labels found in {}", args.from);
        return Ok(());
    }

    // Fetch existing labels in target to avoid duplicates
    let existing = api
        .issue_list_labels()
        .owner(target_owner)
        .repo(target_repo)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let existing_names: std::collections::HashSet<String> = existing
        .iter()
        .filter_map(|l| l.name.clone())
        .collect();

    let mut created = 0;
    let mut skipped = 0;

    for label in &source_labels {
        let name = match &label.name {
            Some(n) => n.clone(),
            None => continue,
        };

        if existing_names.contains(&name) {
            skipped += 1;
            continue;
        }

        let color = label.color.as_deref().unwrap_or("000000").to_string();
        let desc = label.description.clone();

        api.issue_create_label()
            .owner(target_owner)
            .repo(target_repo)
            .body_map(|mut b| {
                b = b.name(name.clone()).color(color.clone());
                if let Some(ref d) = desc {
                    b = b.description(d.clone());
                }
                b
            })
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

        created += 1;
    }

    eprintln!(
        "Cloned {created} label{} from {} ({skipped} already existed)",
        if created == 1 { "" } else { "s" },
        args.from,
    );
    Ok(())
}
