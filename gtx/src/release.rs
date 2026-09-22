use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::paginate;
use crate::repo;

#[derive(Args)]
pub struct ReleaseCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: ReleaseAction,
}

#[derive(Subcommand)]
enum ReleaseAction {
    /// List releases
    List(ListArgs),
    /// Create a release
    Create(CreateArgs),
    /// View a release
    View(ViewArgs),
    /// Download release assets
    Download(DownloadArgs),
    /// Delete a release
    Delete(DeleteArgs),
    /// Edit a release (title, body, draft, prerelease)
    Edit(EditReleaseArgs),
    /// Upload an asset to a release
    Upload(UploadArgs),
    /// Delete a release asset
    DeleteAsset(DeleteAssetArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// Tag name (omit for interactive mode)
    #[arg(short, long)]
    tag: Option<String>,

    /// Release title (defaults to tag name)
    #[arg(short, long)]
    name: Option<String>,

    /// Release body/notes
    #[arg(short, long)]
    body: Option<String>,

    /// Mark as draft
    #[arg(long)]
    draft: bool,

    /// Mark as prerelease
    #[arg(long)]
    prerelease: bool,
}

#[derive(Args)]
struct ViewArgs {
    /// Release ID
    id: i64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct DownloadArgs {
    /// Release ID
    id: i64,
}

#[derive(Args)]
struct DeleteArgs {
    /// Release ID
    id: i64,
}

#[derive(Args)]
struct EditReleaseArgs {
    /// Release ID
    id: i64,

    /// New title
    #[arg(short, long)]
    name: Option<String>,

    /// New body/notes
    #[arg(short, long)]
    body: Option<String>,

    /// Set draft status
    #[arg(long)]
    draft: Option<bool>,

    /// Set prerelease status
    #[arg(long)]
    prerelease: Option<bool>,
}

#[derive(Args)]
struct UploadArgs {
    /// Release ID
    id: i64,

    /// File(s) to upload
    #[arg(required = true)]
    files: Vec<String>,
}

#[derive(Args)]
struct DeleteAssetArgs {
    /// Release ID
    id: i64,

    /// Asset ID
    asset_id: i64,
}

impl ReleaseCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            ReleaseAction::List(args) => list_releases(&self.repo, args).await,
            ReleaseAction::Create(args) => create_release(&self.repo, args).await,
            ReleaseAction::View(args) => view_release(&self.repo, args).await,
            ReleaseAction::Download(args) => download_release(&self.repo, args).await,
            ReleaseAction::Delete(args) => delete_release(&self.repo, args).await,
            ReleaseAction::Edit(args) => edit_release(&self.repo, args).await,
            ReleaseAction::Upload(args) => upload_assets(&self.repo, args).await,
            ReleaseAction::DeleteAsset(args) => delete_asset(&self.repo, args).await,
        }
    }
}

const RELEASE_FIELDS: &[&str] = &[
    "id", "tag_name", "name", "body", "draft", "prerelease",
    "created_at", "published_at", "url", "html_url", "tarball_url", "zipball_url",
    "assets",
];

async fn list_releases(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let releases = paginate::paginate(200, 50, |page, per_page| {
        let api = &api;
        async move {
            Ok(api
                .repo_list_releases()
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
        return crate::json::write_json(&args.json, &releases, &RELEASE_FIELDS);
    }

    if releases.is_empty() {
        eprintln!("No releases found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<20} {:<30} {:<10} {}",
            "ID", "TAG", "TITLE", "STATUS", "PUBLISHED"
        );
    }

    for rel in &releases {
        let id = rel.id.unwrap_or(0);
        let tag = rel.tag_name.as_deref().unwrap_or("");
        let name = rel.name.as_deref().unwrap_or("");
        let truncated_name = if name.len() > 28 {
            format!("{}...", &name[..25])
        } else {
            name.to_string()
        };

        let status = if rel.draft.unwrap_or(false) {
            "draft"
        } else if rel.prerelease.unwrap_or(false) {
            "pre"
        } else {
            "latest"
        };

        let published = rel
            .published_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<6} {:<20} {:<30} {:<10} {}",
            id, tag, truncated_name, status, published
        );
    }

    Ok(())
}

async fn create_release(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let (tag, name, body_text, draft, prerelease) = if let Some(ref tag) = args.tag {
        let name = args.name.clone().unwrap_or_else(|| tag.clone());
        (tag.clone(), name, args.body.clone(), args.draft, args.prerelease)
    } else {
        if !atty_check() {
            eyre::bail!("provide --tag when not running interactively");
        }
        interactive_create_release()?
    };

    let release = api
        .repo_create_release()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            b = b
                .tag_name(tag.clone())
                .name(name.clone())
                .draft(draft)
                .prerelease(prerelease);
            if let Some(ref body) = body_text {
                b = b.body(body.clone());
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = release.id.unwrap_or(0);
    let url = release.html_url.as_deref().unwrap_or("");
    eprintln!("Created release #{id}: {url}");
    Ok(())
}

fn interactive_create_release() -> Result<(String, String, Option<String>, bool, bool)> {
    let tag = inquire::Text::new("Tag name:")
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid("Tag is required".into()))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;

    let name = inquire::Text::new("Release title:")
        .with_default(&tag)
        .prompt()?;

    let body = crate::prompt::edit_body("")?;
    let body_opt = if body.is_empty() { None } else { Some(body) };

    let draft = inquire::Confirm::new("Draft?").with_default(false).prompt()?;
    let prerelease = inquire::Confirm::new("Prerelease?").with_default(false).prompt()?;

    let action = inquire::Select::new("What's next?", vec!["Submit", "Cancel"]).prompt()?;
    if action == "Cancel" {
        eyre::bail!("Cancelled");
    }

    Ok((tag, name, body_opt, draft, prerelease))
}

async fn view_release(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = api
        .repo_get_release()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rel)?);
        return Ok(());
    }

    let name = rel.name.as_deref().unwrap_or("(no title)");
    let tag = rel.tag_name.as_deref().unwrap_or("");
    let id = rel.id.unwrap_or(0);

    let status = if rel.draft.unwrap_or(false) {
        "draft"
    } else if rel.prerelease.unwrap_or(false) {
        "prerelease"
    } else {
        "release"
    };

    println!("{name} (#{id})");
    println!("Tag: {tag} -- {status}");

    if let Some(ref author) = rel.author {
        let login = author.login.as_deref().unwrap_or("unknown");
        println!("Author: {login}");
    }

    if let Some(published) = rel.published_at {
        println!("Published: {}", relative_time(published));
    }

    // Body
    if let Some(ref body) = rel.body {
        if !body.is_empty() {
            println!();
            println!("{body}");
        }
    }

    // Assets
    if !rel.assets.is_empty() {
        println!("\nAssets:");
        for asset in &rel.assets {
            let name = asset.name.as_deref().unwrap_or("unnamed");
            let downloads = asset.download_count.unwrap_or(0);
            println!("  {name} ({downloads} downloads)");
        }
    }

    // URL
    if let Some(ref url) = rel.html_url {
        println!();
        println!("{url}");
    }

    Ok(())
}

async fn download_release(repo_args: &repo::RepoArgs, args: &DownloadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = api
        .repo_get_release()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if rel.assets.is_empty() {
        eprintln!("No assets to download for release #{}", args.id);
        return Ok(());
    }

    for asset in &rel.assets {
        let url = asset
            .browser_download_url
            .as_deref()
            .ok_or_else(|| eyre::eyre!("Asset has no download URL"))?;
        let filename = asset.name.as_deref().unwrap_or("download");

        eprintln!("Downloading {filename}...");

        // Strip base URL prefix to get the API path for raw_request
        let path = url
            .strip_prefix(api.base_url())
            .map(|p| p.to_string())
            .unwrap_or_else(|| url.to_string());
        let resp = api
            .raw_request(gitea_api::Method::GET, &path, None)
            .await
            .map_err(|e| eyre::eyre!("Download failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            eyre::bail!(
                "Download failed: {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            );
        }

        let bytes = resp.bytes().await?;
        std::fs::write(filename, &bytes)?;
        eprintln!("  Saved {filename} ({} bytes)", bytes.len());
    }

    Ok(())
}

async fn delete_release(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_delete_release()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Release #{} deleted", args.id);
    Ok(())
}

async fn edit_release(repo_args: &repo::RepoArgs, args: &EditReleaseArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_edit_release()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .body_map(|mut b| {
            if let Some(ref name) = args.name {
                b = b.name(name.clone());
            }
            if let Some(ref body) = args.body {
                b = b.body(body.clone());
            }
            if let Some(draft) = args.draft {
                b = b.draft(draft);
            }
            if let Some(prerelease) = args.prerelease {
                b = b.prerelease(prerelease);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Updated release #{}", args.id);
    Ok(())
}

async fn upload_assets(repo_args: &repo::RepoArgs, args: &UploadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    for file_path in &args.files {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            eyre::bail!("File not found: {file_path}");
        }

        let filename = path
            .file_name()
            .ok_or_else(|| eyre::eyre!("Invalid filename: {file_path}"))?
            .to_string_lossy()
            .to_string();

        let file_bytes = std::fs::read(path)?;

        // Gitea expects multipart form upload — use raw reqwest
        let url = api.url_for(&format!(
            "repos/{owner}/{repo}/releases/{}/assets?name={filename}",
            args.id
        ));

        let part = reqwest::multipart::Part::bytes(file_bytes).file_name(filename.clone());
        let form = reqwest::multipart::Form::new().part("attachment", part);

        let resp = reqwest::Client::new()
            .post(&url)
            .header(
                "Authorization",
                format!("token {}", config.token),
            )
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let message = parsed["message"]
                .as_str()
                .unwrap_or(status.canonical_reason().unwrap_or("Error"));
            eyre::bail!("HTTP {}: {message}", status.as_u16());
        }

        eprintln!("Uploaded {filename} to release #{}", args.id);
    }

    Ok(())
}

async fn delete_asset(repo_args: &repo::RepoArgs, args: &DeleteAssetArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    api.repo_delete_release_attachment()
        .owner(owner)
        .repo(repo)
        .id(args.id)
        .attachment_id(args.asset_id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted asset #{} from release #{}", args.asset_id, args.id);
    Ok(())
}
