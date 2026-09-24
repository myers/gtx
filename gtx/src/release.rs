use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use crate::paginate;
use crate::repo;
use gitea_api::types::Release;

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
    /// Edit a release
    Edit(EditReleaseArgs),
    /// Upload assets to a release
    Upload(UploadArgs),
    /// Delete a release asset
    DeleteAsset(DeleteAssetArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Maximum number of items to fetch
    #[arg(short = 'L', long, default_value_t = 30)]
    limit: i64,

    /// Exclude draft releases
    #[arg(long)]
    exclude_drafts: bool,

    /// Exclude pre-releases
    #[arg(long)]
    exclude_pre_releases: bool,

    /// Order of releases returned
    #[arg(short = 'O', long, value_parser = ["asc", "desc"], default_value = "desc")]
    order: String,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// Tag name (prompted for when omitted and running interactively)
    tag: Option<String>,

    /// Files to upload as release assets. gh's `file#label` display labels
    /// are dropped with a warning: Gitea assets have no label
    files: Vec<String>,

    /// Release title (defaults to the tag name)
    #[arg(short, long)]
    title: Option<String>,

    /// Release notes
    #[arg(short, long, conflicts_with = "notes_file")]
    notes: Option<String>,

    /// Read release notes from file (use "-" to read from standard input)
    #[arg(short = 'F', long, value_name = "FILE")]
    notes_file: Option<String>,

    /// Save the release as a draft instead of publishing it
    #[arg(short, long)]
    draft: bool,

    /// Mark the release as a prerelease
    #[arg(short, long)]
    prerelease: bool,

    /// Target branch or full commit SHA [default: the default branch]
    #[arg(long, value_name = "BRANCH")]
    target: Option<String>,

    /// Abort in case the git tag doesn't already exist in the remote repository
    #[arg(long)]
    verify_tag: bool,

    /// Fetch notes from the tag annotation or message of commit associated with tag
    #[arg(long, conflicts_with_all = ["generate_notes", "notes_start_tag"])]
    notes_from_tag: bool,

    /// Mark this release as "Latest" (`--latest=false` to explicitly NOT set as latest).
    /// Gitea can't mark releases: its latest is always the most recently created
    /// published non-prerelease, so this only checks that the new release ends up
    /// latest (or, with `=false`, that it can't be)
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    latest: Option<bool>,

    /// Not supported: Gitea has no API for generating release notes
    #[arg(long)]
    generate_notes: bool,

    /// Not supported: Gitea has no API for generating release notes
    #[arg(long, value_name = "STRING")]
    notes_start_tag: Option<String>,

    /// Fail if there are no commits since the last release (no impact on the first release)
    #[arg(long)]
    fail_on_no_commits: bool,

    /// Not supported: Gitea has no discussions
    #[arg(long, value_name = "STRING")]
    discussion_category: Option<String>,
}

#[derive(Args)]
struct ViewArgs {
    /// Release tag (the latest release when omitted)
    tag: Option<String>,

    #[command(flatten)]
    json: crate::json::JsonArgs,

    /// Open the release in the browser
    #[arg(short, long)]
    web: bool,
}

#[derive(Args)]
struct DownloadArgs {
    /// Release tag (the latest release when omitted; then `--pattern` or `--archive` is required)
    #[arg(required_unless_present_any = ["pattern", "archive"])]
    tag: Option<String>,

    /// Download only assets that match a glob pattern (repeatable)
    #[arg(short, long)]
    pattern: Vec<String>,

    /// Download the source code archive in the specified format
    #[arg(short = 'A', long, value_parser = ["zip", "tar.gz"], conflicts_with = "pattern")]
    archive: Option<String>,

    /// The directory to download files into [default: .]
    #[arg(short = 'D', long)]
    dir: Option<String>,

    /// The file to write a single asset to (use "-" to write to standard output)
    #[arg(short = 'O', long, conflicts_with = "dir")]
    output: Option<String>,

    /// Overwrite existing files of the same name
    #[arg(long)]
    clobber: bool,

    /// Skip downloading when files of the same name exist
    #[arg(long, conflicts_with = "clobber")]
    skip_existing: bool,
}

#[derive(Args)]
struct DeleteArgs {
    /// Release tag
    tag: String,

    /// Skip the confirmation prompt
    #[arg(short, long)]
    yes: bool,

    /// Delete the specified tag in addition to its release
    #[arg(long)]
    cleanup_tag: bool,
}

#[derive(Args)]
struct EditReleaseArgs {
    /// Release tag
    #[arg(value_name = "TAG")]
    release: String,

    /// Release title
    #[arg(short, long)]
    title: Option<String>,

    /// Release notes
    #[arg(short, long, conflicts_with = "notes_file")]
    notes: Option<String>,

    /// Read release notes from file (use "-" to read from standard input)
    #[arg(short = 'F', long, value_name = "FILE")]
    notes_file: Option<String>,

    /// Save the release as a draft instead of publishing it (`--draft=false` publishes it)
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    draft: Option<bool>,

    /// Mark the release as a prerelease (`--prerelease=false` unmarks it)
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    prerelease: Option<bool>,

    /// The name of the tag
    #[arg(long = "tag", value_name = "STRING")]
    new_tag: Option<String>,

    /// Target branch or full commit SHA
    #[arg(long, value_name = "BRANCH")]
    target: Option<String>,

    /// Abort in case the git tag doesn't already exist in the remote repository
    #[arg(long)]
    verify_tag: bool,

    /// Explicitly mark the release as "Latest" (`--latest=false` to require it isn't).
    /// Gitea can't mark releases: its latest is always the most recently created
    /// published non-prerelease, so this only checks that the edited release is
    /// (or isn't) latest
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    latest: Option<bool>,

    /// Not supported: Gitea has no discussions
    #[arg(long, value_name = "STRING")]
    discussion_category: Option<String>,
}

#[derive(Args)]
struct UploadArgs {
    /// Release tag
    tag: String,

    /// File(s) to upload. gh's `file#label` display labels are dropped with a
    /// warning: Gitea assets have no label
    #[arg(required = true)]
    files: Vec<String>,

    /// Overwrite existing assets of the same name
    #[arg(long)]
    clobber: bool,
}

#[derive(Args)]
struct DeleteAssetArgs {
    /// Release tag
    tag: String,

    /// Asset name
    asset_name: String,

    /// Skip the confirmation prompt
    #[arg(short, long)]
    yes: bool,
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

/// A listed release plus whether Gitea considers it the latest.
struct ListedRelease {
    rel: Release,
    latest: bool,
}

/// gh's `release list --json` fields.
const LIST_FIELDS: &[Field<ListedRelease>] = &[
    field("createdAt", |r| gh::time(r.rel.created_at)),
    field("isDraft", |r| gh::v(r.rel.draft.unwrap_or(false))),
    field("isImmutable", |_| gh::v(false)),
    field("isLatest", |r| gh::v(r.latest)),
    field("isPrerelease", |r| gh::v(r.rel.prerelease.unwrap_or(false))),
    field("name", |r| gh::v(r.rel.name.as_deref().unwrap_or(""))),
    field("publishedAt", |r| gh::time(r.rel.published_at)),
    field("tagName", |r| gh::v(&r.rel.tag_name)),
];

/// gh's `release view --json` fields. Gitea has no GraphQL node IDs, so
/// `id` is the numeric ID, same as `databaseId`.
const VIEW_FIELDS: &[Field<Release>] = &[
    field("apiUrl", |r| gh::v(&r.url)),
    field("assets", |r| {
        serde_json::Value::Array(
            r.assets
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "size": a.size,
                        "downloadCount": a.download_count,
                        "createdAt": gh::time(a.created_at),
                        "url": a.browser_download_url,
                    })
                })
                .collect(),
        )
    }),
    field("author", |r| gh::user(r.author.as_ref())),
    field("body", |r| gh::v(r.body.as_deref().unwrap_or(""))),
    field("createdAt", |r| gh::time(r.created_at)),
    field("databaseId", |r| gh::v(r.id)),
    field("id", |r| gh::v(r.id)),
    field("isDraft", |r| gh::v(r.draft.unwrap_or(false))),
    field("isImmutable", |_| gh::v(false)),
    field("isPrerelease", |r| gh::v(r.prerelease.unwrap_or(false))),
    field("name", |r| gh::v(r.name.as_deref().unwrap_or(""))),
    field("publishedAt", |r| gh::time(r.published_at)),
    field("tagName", |r| gh::v(&r.tag_name)),
    field("tarballUrl", |r| gh::v(&r.tarball_url)),
    field("targetCommitish", |r| gh::v(&r.target_commitish)),
    field("uploadUrl", |r| gh::v(&r.upload_url)),
    field("url", |r| gh::v(&r.html_url)),
    field("zipballUrl", |r| gh::v(&r.zipball_url)),
];

async fn list_releases(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let json = args.json.select(LIST_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Gitea lists newest first and can't sort the other way, so for `asc`
    // fetch everything and keep the oldest `limit`.
    let asc = args.order == "asc";
    let fetch_limit = if asc { i64::MAX } else { args.limit };
    let mut releases = paginate::paginate(fetch_limit, 50, |page, per_page| {
        let api = &api;
        async move {
            let mut req = api
                .repo_list_releases()
                .owner(owner)
                .repo(repo)
                .page(page)
                .limit(per_page);
            if args.exclude_drafts {
                req = req.draft(false);
            }
            if args.exclude_pre_releases {
                req = req.pre_release(false);
            }
            Ok(req.send().await.map_err(api_error)?.into_inner())
        }
    })
    .await?;
    if asc {
        releases.reverse();
        releases.truncate(args.limit.max(0) as usize);
    }

    if let Some(json) = json {
        let latest_id = if json.wants("isLatest") {
            latest_release(&api, owner, repo).await?.and_then(|r| r.id)
        } else {
            None
        };
        let rows: Vec<ListedRelease> = releases
            .into_iter()
            .map(|rel| ListedRelease {
                latest: rel.id.is_some() && rel.id == latest_id,
                rel,
            })
            .collect();
        return json.write_list(&rows);
    }

    let is_tty = atty_check();
    if releases.is_empty() {
        // Like gh: not a failure, and only worth saying to a person.
        if is_tty {
            eprintln!("no releases found");
        }
        return Ok(());
    }

    let latest_id = latest_release(&api, owner, repo).await?.and_then(|r| r.id);
    let rows: Vec<[String; 4]> = releases
        .iter()
        .map(|rel| {
            let tag = rel.tag_name.clone().unwrap_or_default();
            let name = rel.name.as_deref().unwrap_or("");
            let title = name.split_whitespace().collect::<Vec<_>>().join(" ");
            let title = if title.is_empty() { tag.clone() } else { title };
            let badge = if rel.id.is_some() && rel.id == latest_id {
                "Latest"
            } else if rel.draft.unwrap_or(false) {
                "Draft"
            } else if rel.prerelease.unwrap_or(false) {
                "Pre-release"
            } else {
                ""
            };
            let when = rel.published_at.or(rel.created_at);
            let published = match when {
                Some(dt) if is_tty => relative_time(dt),
                Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                None => String::new(),
            };
            [title, badge.to_string(), tag, published]
        })
        .collect();

    if !is_tty {
        for row in &rows {
            println!("{}", row.join("\t"));
        }
        return Ok(());
    }

    let header = ["TITLE", "TYPE", "TAG NAME", "PUBLISHED"];
    let width = |i: usize| {
        rows.iter()
            .map(|r| r[i].chars().count())
            .chain([header[i].len()])
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    let line = |cols: [&str; 4]| {
        format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {}",
            cols[0],
            cols[1],
            cols[2],
            cols[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2]
        )
    };
    println!("{}", line(header));
    for [title, badge, tag, published] in &rows {
        println!("{}", line([title, badge, tag, published]).trim_end());
    }
    Ok(())
}

/// The release Gitea considers latest (the newest published non-prerelease), if any.
async fn latest_release(api: &gitea_api::Gitea, owner: &str, repo: &str) -> Result<Option<Release>> {
    match api
        .repo_get_latest_release()
        .owner(owner)
        .repo(repo)
        .send()
        .await
    {
        Ok(rel) => Ok(Some(rel.into_inner())),
        Err(e) => match gitea_api::GiteaError::from(e) {
            gitea_api::GiteaError::Api { status: 404, .. } => Ok(None),
            e => Err(eyre::eyre!("{e}")),
        },
    }
}

/// The release tagged `tag`, or the latest release when `tag` is `None`.
///
/// Gitea's by-tag endpoint can miss drafts, so, like gh, a 404 there falls
/// back to searching the repository's draft releases for the tag.
async fn fetch_release(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    tag: Option<&str>,
) -> Result<Release> {
    let Some(tag) = tag else {
        return Ok(api
            .repo_get_latest_release()
            .owner(owner)
            .repo(repo)
            .send()
            .await
            .map_err(api_error)?
            .into_inner());
    };
    match api
        .repo_get_release_by_tag()
        .owner(owner)
        .repo(repo)
        .tag(tag)
        .send()
        .await
    {
        Ok(rel) => return Ok(rel.into_inner()),
        Err(e) => match gitea_api::GiteaError::from(e) {
            gitea_api::GiteaError::Api { status: 404, .. } => {}
            e => return Err(eyre::eyre!("{e}")),
        },
    }
    const PER_PAGE: i64 = 50;
    for page in 1.. {
        let drafts = api
            .repo_list_releases()
            .owner(owner)
            .repo(repo)
            .draft(true)
            .page(page)
            .limit(PER_PAGE)
            .send()
            .await
            .map_err(api_error)?
            .into_inner();
        let last = (drafts.len() as i64) < PER_PAGE;
        if let Some(rel) = drafts
            .into_iter()
            .find(|r| r.tag_name.as_deref() == Some(tag))
        {
            return Ok(rel);
        }
        if last {
            break;
        }
    }
    eyre::bail!("release not found")
}

fn api_error(e: impl Into<gitea_api::GiteaError>) -> eyre::Report {
    eyre::eyre!("{}", e.into())
}

/// Whether we can ask the user questions (stdin and stdout are terminals), as gh's `CanPrompt`.
fn can_prompt() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin()) && atty_check()
}

/// Release notes from `--notes`, or `--notes-file` ("-" for stdin).
fn read_notes(notes: &Option<String>, notes_file: &Option<String>) -> Result<Option<String>> {
    Ok(match (notes, notes_file.as_deref()) {
        (Some(notes), _) => Some(notes.clone()),
        (None, Some("-")) => Some(std::io::read_to_string(std::io::stdin())?),
        (None, Some(path)) => {
            Some(std::fs::read_to_string(path).map_err(|e| eyre::eyre!("{path}: {e}"))?)
        }
        (None, None) => None,
    })
}

/// `--verify-tag`: fail unless git tag `tag` exists on the server.
async fn verify_tag(api: &gitea_api::Gitea, owner: &str, repo: &str, tag: &str) -> Result<()> {
    match api
        .repo_get_tag()
        .owner(owner)
        .repo(repo)
        .tag(tag)
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => match gitea_api::GiteaError::from(e) {
            gitea_api::GiteaError::Api { status: 404, .. } => eyre::bail!(
                "tag {tag} doesn't exist in the repo {owner}/{repo}, aborting due to --verify-tag flag"
            ),
            e => Err(eyre::eyre!("{e}")),
        },
    }
}

/// The paths of asset arguments, with gh's `file#Display label` labels
/// dropped (with a warning: Gitea attachments have no label). Fails on the
/// first that isn't a readable regular file.
fn asset_paths(files: &[String]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for arg in files {
        let path = match arg.find('#') {
            Some(i) if i > 0 => {
                let label = &arg[i + 1..];
                eprintln!(
                    "warning: Gitea release assets have no display label; ignoring {label:?} for {}",
                    &arg[..i]
                );
                &arg[..i]
            }
            _ => arg.as_str(),
        };
        if !std::path::Path::new(path).is_file() {
            eyre::bail!("File not found: {path}");
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

const NO_DISCUSSIONS: &str = "--discussion-category is not supported: Gitea has no discussions";

fn file_name(path: &str) -> Result<String> {
    Ok(std::path::Path::new(path)
        .file_name()
        .ok_or_else(|| eyre::eyre!("Invalid filename: {path}"))?
        .to_string_lossy()
        .into_owned())
}

/// Upload the file at `path` as an asset of release `release_id`.
async fn upload_asset(
    api: &gitea_api::Gitea,
    config: &Config,
    owner: &str,
    repo: &str,
    release_id: i64,
    path: &str,
) -> Result<()> {
    let name = file_name(path)?;
    let mut url = url::Url::parse(&api.url_for(&format!(
        "repos/{owner}/{repo}/releases/{release_id}/assets"
    )))?;
    url.query_pairs_mut().append_pair("name", &name);

    let part = reqwest::multipart::Part::bytes(std::fs::read(path)?).file_name(name);
    let form = reqwest::multipart::Form::new().part("attachment", part);
    let resp = reqwest::Client::new()
        .post(url)
        .header("Authorization", format!("token {}", config.token))
        .multipart(form)
        .send()
        .await?;
    gitea_api::error_for_status(resp)
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    Ok(())
}

fn confirm(question: &str) -> Result<()> {
    if !inquire::Confirm::new(question)
        .with_default(false)
        .prompt()?
    {
        eyre::bail!("Cancelled");
    }
    Ok(())
}

async fn create_release(repo_args: &repo::RepoArgs, args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    if args.generate_notes || args.notes_start_tag.is_some() {
        eyre::bail!(
            "--generate-notes and --notes-start-tag are not supported: Gitea has no API for generating release notes"
        );
    }
    if args.discussion_category.is_some() {
        eyre::bail!(NO_DISCUSSIONS);
    }
    let files = asset_paths(&args.files)?;
    let notes = read_notes(&args.notes, &args.notes_file)?;
    let mut new = match &args.tag {
        Some(tag) => NewRelease {
            tag: tag.clone(),
            title: args.title.clone().unwrap_or_else(|| tag.clone()),
            notes,
            draft: args.draft,
            prerelease: args.prerelease,
        },
        None => {
            if !can_prompt() {
                eyre::bail!("tag required when not running interactively");
            }
            interactive_create_release(args, notes)?
        }
    };

    // Gitea has no "make latest": its latest release is always the newest
    // published non-prerelease one.
    let can_be_latest = !new.draft && !new.prerelease;
    match args.latest {
        Some(true) if !can_be_latest => eyre::bail!(
            "--latest can't be used with a draft or prerelease: Gitea only treats published, non-prerelease releases as latest"
        ),
        Some(false) if can_be_latest => eyre::bail!(
            "--latest=false is not supported: Gitea always treats the newest published non-prerelease release as latest"
        ),
        _ => {}
    }

    if args.verify_tag {
        verify_tag(&api, owner, repo, &new.tag).await?;
    }

    if args.fail_on_no_commits {
        fail_on_no_commits(&api, owner, repo, args.target.as_deref()).await?;
    }

    if args.notes_from_tag {
        let message = tag_message(&api, owner, repo, &new.tag).await?;
        new.notes = Some(match new.notes.take().filter(|n| !n.is_empty()) {
            Some(notes) => format!("{notes}\n{message}"),
            None => message,
        });
    }

    // Like gh: attach the files to a draft, then publish it once they're all there.
    let publish_after_upload = !files.is_empty() && !new.draft;
    let rel = api
        .repo_create_release()
        .owner(owner)
        .repo(repo)
        .body_map(|mut b| {
            b = b
                .tag_name(new.tag.clone())
                .name(new.title.clone())
                .draft(new.draft || publish_after_upload)
                .prerelease(new.prerelease);
            if let Some(notes) = &new.notes {
                b = b.body(notes.clone());
            }
            if let Some(target) = &args.target {
                b = b.target_commitish(target.clone());
            }
            b
        })
        .send()
        .await
        .map_err(api_error)?
        .into_inner();
    let id = rel
        .id
        .ok_or_else(|| eyre::eyre!("server returned a release without an id"))?;

    for file in &files {
        upload_asset(&api, &config, owner, repo, id, file).await?;
    }

    let rel = if publish_after_upload {
        api.repo_edit_release()
            .owner(owner)
            .repo(repo)
            .id(id)
            .body_map(|b| b.draft(false))
            .send()
            .await
            .map_err(api_error)?
            .into_inner()
    } else {
        rel
    };

    println!("{}", rel.html_url.as_deref().unwrap_or(""));

    if args.latest == Some(true) {
        check_latest(&api, owner, repo, &rel, true).await?;
    }
    Ok(())
}

/// `--latest[=false]`: fail unless `rel` is (or, for `want = false`, isn't)
/// Gitea's latest release, which Gitea picks by itself.
async fn check_latest(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    rel: &Release,
    want: bool,
) -> Result<()> {
    const WHY: &str = "Gitea can't mark a release latest; it picks the published non-prerelease created most recently (a release of an existing tag counts as created at the tag's commit)";
    let latest = latest_release(api, owner, repo).await?;
    let is_latest = latest.as_ref().and_then(|r| r.id) == rel.id;
    let tag = rel.tag_name.as_deref().unwrap_or("");
    if want && !is_latest {
        let latest_tag = latest
            .as_ref()
            .and_then(|r| r.tag_name.as_deref())
            .unwrap_or("none");
        eyre::bail!("Gitea's latest release is {latest_tag}, not {tag}: {WHY}");
    }
    if !want && is_latest {
        eyre::bail!("{tag} is still Gitea's latest release: {WHY}");
    }
    Ok(())
}

/// `--fail-on-no-commits`: fail if `target` (default: the default branch)
/// has no commits since the latest release's tag. No latest release, no check.
async fn fail_on_no_commits(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    target: Option<&str>,
) -> Result<()> {
    let Some(latest) = latest_release(api, owner, repo).await? else {
        return Ok(());
    };
    let Some(latest_tag) = latest.tag_name.filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    let target = match target {
        Some(target) => target.to_string(),
        None => api
            .repo_get()
            .owner(owner)
            .repo(repo)
            .send()
            .await
            .map_err(api_error)?
            .into_inner()
            .default_branch
            .ok_or_else(|| eyre::eyre!("repository {owner}/{repo} has no default branch"))?,
    };
    let compare = api
        .repo_compare_diff()
        .owner(owner)
        .repo(repo)
        .basehead(format!("{latest_tag}...{target}"))
        .send()
        .await
        .map_err(api_error)?
        .into_inner();
    let ahead = compare
        .total_commits
        .unwrap_or(compare.commits.len() as i64);
    if ahead < 1 {
        eyre::bail!("no new commits since the last release: {latest_tag}");
    }
    Ok(())
}

/// `--notes-from-tag`: the tag's annotation, or for a lightweight tag the
/// message of its commit (Gitea's tag API returns whichever applies).
async fn tag_message(api: &gitea_api::Gitea, owner: &str, repo: &str, tag: &str) -> Result<String> {
    match api
        .repo_get_tag()
        .owner(owner)
        .repo(repo)
        .tag(tag)
        .send()
        .await
    {
        Ok(t) => Ok(t.into_inner().message.unwrap_or_default()),
        Err(e) => match gitea_api::GiteaError::from(e) {
            gitea_api::GiteaError::Api { status: 404, .. } => eyre::bail!(
                "cannot generate release notes from tag {tag} as it does not exist in the repo {owner}/{repo}"
            ),
            e => Err(eyre::eyre!("{e}")),
        },
    }
}

/// What `release create` sends, from flags or prompts.
struct NewRelease {
    tag: String,
    title: String,
    notes: Option<String>,
    draft: bool,
    prerelease: bool,
}

fn interactive_create_release(args: &CreateArgs, notes: Option<String>) -> Result<NewRelease> {
    let tag = inquire::Text::new("Tag name:")
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid(
                    "Tag is required".into(),
                ))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;

    let title = match &args.title {
        Some(title) => title.clone(),
        None => inquire::Text::new("Title:").with_default(&tag).prompt()?,
    };

    let notes = match notes {
        Some(notes) => Some(notes),
        // Like gh, the tag's message stands in for notes the user would type.
        None if args.notes_from_tag => None,
        None => Some(crate::prompt::edit_body("")?).filter(|n| !n.is_empty()),
    };

    let prerelease = args.prerelease
        || inquire::Confirm::new("Is this a prerelease?")
            .with_default(false)
            .prompt()?;

    let options = vec!["Publish release", "Save as draft", "Cancel"];
    let draft = match inquire::Select::new("Submit?", options).prompt()? {
        "Cancel" => eyre::bail!("Cancelled"),
        choice => args.draft || choice == "Save as draft",
    };

    Ok(NewRelease {
        tag,
        title,
        notes,
        draft,
        prerelease,
    })
}

async fn view_release(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let json = args.json.select(VIEW_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = fetch_release(&api, owner, repo, args.tag.as_deref()).await?;

    if args.web {
        let url = rel.html_url.as_deref().unwrap_or("");
        if atty_check() {
            eprintln!("Opening {url} in your browser.");
        }
        return crate::browse::open_url(url);
    }

    if let Some(json) = json {
        return json.write_one(&rel);
    }

    if atty_check() {
        print_release_tty(&rel);
    } else {
        print_release_plain(&rel);
    }
    Ok(())
}

/// gh's machine-readable `release view` output.
fn print_release_plain(rel: &Release) {
    let draft = rel.draft.unwrap_or(false);
    println!("title:\t{}", rel.name.as_deref().unwrap_or(""));
    println!("tag:\t{}", rel.tag_name.as_deref().unwrap_or(""));
    println!("draft:\t{draft}");
    println!("prerelease:\t{}", rel.prerelease.unwrap_or(false));
    let author = rel.author.as_ref().and_then(|a| a.login.as_deref());
    println!("author:\t{}", author.unwrap_or(""));
    if let Some(created) = rel.created_at {
        println!(
            "created:\t{}",
            created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
    }
    if let (false, Some(published)) = (draft, rel.published_at) {
        println!(
            "published:\t{}",
            published.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
    }
    println!("url:\t{}", rel.html_url.as_deref().unwrap_or(""));
    for asset in &rel.assets {
        println!("asset:\t{}", asset.name.as_deref().unwrap_or(""));
    }
    println!("--");
    let body = rel.body.as_deref().unwrap_or("");
    print!("{body}");
    if !body.ends_with('\n') {
        println!();
    }
}

fn print_release_tty(rel: &Release) {
    let name = rel.name.as_deref().unwrap_or("(no title)");
    let tag = rel.tag_name.as_deref().unwrap_or("");

    let status = if rel.draft.unwrap_or(false) {
        " (Draft)"
    } else if rel.prerelease.unwrap_or(false) {
        " (Pre-release)"
    } else {
        ""
    };
    println!("{name}{status}");
    println!("Tag: {tag}");

    let author = rel.author.as_ref().and_then(|a| a.login.as_deref());
    match (author, rel.published_at) {
        (Some(login), Some(published)) => {
            println!("{login} released this {}", relative_time(published))
        }
        (Some(login), None) => println!("{login} created this"),
        _ => {}
    }

    if let Some(body) = rel.body.as_deref().filter(|b| !b.is_empty()) {
        println!();
        println!("{body}");
    }

    if !rel.assets.is_empty() {
        println!("\nAssets");
        for asset in &rel.assets {
            let name = asset.name.as_deref().unwrap_or("unnamed");
            let size = asset.size.unwrap_or(0);
            println!("  {name}  {size} bytes");
        }
    }

    if let Some(url) = &rel.html_url {
        println!();
        println!("View on Gitea: {url}");
    }
}

/// What `release download` fetches: a named asset, or the source archive
/// whose name comes from the response's `Content-Disposition`.
struct Download {
    url: String,
    name: Option<String>,
}

async fn download_release(repo_args: &repo::RepoArgs, args: &DownloadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = fetch_release(&api, owner, repo, args.tag.as_deref()).await?;
    let tag = rel.tag_name.as_deref().unwrap_or("");

    let downloads = if let Some(format) = &args.archive {
        let url = if format == "zip" { &rel.zipball_url } else { &rel.tarball_url };
        let url = url
            .clone()
            .ok_or_else(|| eyre::eyre!("Release {tag} has no {format} archive URL"))?;
        vec![Download { url, name: None }]
    } else {
        let patterns = args
            .pattern
            .iter()
            .map(|p| glob::Pattern::new(p).map_err(|e| eyre::eyre!("invalid pattern {p:?}: {e}")))
            .collect::<Result<Vec<_>>>()?;
        let mut downloads = Vec::new();
        for asset in &rel.assets {
            let name = asset.name.as_deref().unwrap_or("");
            if !patterns.is_empty() && !patterns.iter().any(|p| p.matches(name)) {
                continue;
            }
            let url = asset
                .browser_download_url
                .clone()
                .ok_or_else(|| eyre::eyre!("Asset {name} has no download URL"))?;
            downloads.push(Download { url, name: Some(name.to_string()) });
        }
        if downloads.is_empty() {
            if rel.assets.is_empty() {
                eyre::bail!("no assets to download");
            }
            eyre::bail!("no assets match the file pattern");
        }
        downloads
    };

    if args.output.is_some() && downloads.len() > 1 {
        eyre::bail!(
            "unable to write more than one asset with `--output`, got {} assets",
            downloads.len()
        );
    }

    let dir = std::path::Path::new(args.dir.as_deref().unwrap_or(""));
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(dir)?;
    }

    for download in downloads {
        // Asset names are known up front; check before fetching anything.
        if let Some(name) = &download.name {
            let dest = destination(args, dir, name)?;
            if !should_write(args, dest.as_deref())? {
                continue;
            }
        }

        let resp = api
            .download(&download.url)
            .await
            .map_err(|e| eyre::eyre!("Download failed: {e}"))?;
        let resp = gitea_api::error_for_status(resp)
            .await
            .map_err(|e| eyre::eyre!("Download failed: {e}"))?;

        let name = match download.name {
            Some(name) => name,
            None => content_disposition_filename(resp.headers()).unwrap_or_else(|| {
                let ext = args.archive.as_deref().unwrap_or("zip");
                format!("{repo}-{tag}.{ext}")
            }),
        };
        let dest = destination(args, dir, &name)?;
        if !should_write(args, dest.as_deref())? {
            continue;
        }

        let bytes = resp.bytes().await?;
        match dest {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, &bytes)?;
            }
            None => {
                use std::io::Write;
                std::io::stdout().write_all(&bytes)?;
            }
        }
    }

    Ok(())
}

/// Where a download named `name` goes: `--output` if given (`None` for
/// stdout), else `<dir>/<name>`, refusing names that would land elsewhere.
fn destination(
    args: &DownloadArgs,
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<std::path::PathBuf>> {
    match args.output.as_deref() {
        Some("-") => return Ok(None),
        Some(output) => return Ok(Some(output.into())),
        None => {}
    }
    let mut parts = std::path::Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(Some(dir.join(name))),
        _ => Err(eyre::eyre!("Refusing to download asset with unsafe name {name:?}")),
    }
}

/// Whether to write `dest`, per `--clobber` / `--skip-existing` (like `gh`,
/// an existing file is an error when neither is given).
fn should_write(args: &DownloadArgs, dest: Option<&std::path::Path>) -> Result<bool> {
    let Some(dest) = dest.filter(|d| d.exists()) else {
        return Ok(true);
    };
    if args.skip_existing {
        return Ok(false);
    }
    if !args.clobber {
        eyre::bail!(
            "{} already exists (use `--clobber` to overwrite file or `--skip-existing` to skip file)",
            dest.display()
        );
    }
    Ok(true)
}

/// The `filename` from a `Content-Disposition: attachment; filename="..."` header.
fn content_disposition_filename(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers.get(reqwest::header::CONTENT_DISPOSITION)?.to_str().ok()?;
    value.split(';').find_map(|part| {
        let name = part.trim().strip_prefix("filename=")?;
        Some(name.trim_matches('"').to_string()).filter(|n| !n.is_empty())
    })
}

async fn delete_release(repo_args: &repo::RepoArgs, args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = fetch_release(&api, owner, repo, Some(&args.tag)).await?;
    let id = rel
        .id
        .ok_or_else(|| eyre::eyre!("release {} has no id", args.tag))?;
    let draft = rel.draft.unwrap_or(false);

    if !args.yes && can_prompt() {
        confirm(&format!("Delete release {} in {owner}/{repo}?", args.tag))?;
    }

    api.repo_delete_release()
        .owner(owner)
        .repo(repo)
        .id(id)
        .send()
        .await
        .map_err(api_error)?;

    if args.cleanup_tag {
        let deleted = api
            .repo_delete_tag()
            .owner(owner)
            .repo(repo)
            .tag(&args.tag)
            .send()
            .await;
        match deleted.map_err(gitea_api::GiteaError::from) {
            Ok(_) => {}
            // A draft's tag need not exist yet.
            Err(gitea_api::GiteaError::Api { status: 404, .. }) if draft => {}
            Err(e) => eyre::bail!("{e}"),
        }
    }

    if atty_check() {
        if args.cleanup_tag {
            eprintln!("✓ Deleted release and tag {}", args.tag);
        } else {
            eprintln!("✓ Deleted release {}", args.tag);
            if !draft {
                eprintln!(
                    "! Note that the {} git tag still remains in the repository",
                    args.tag
                );
            }
        }
    }
    Ok(())
}

async fn edit_release(repo_args: &repo::RepoArgs, args: &EditReleaseArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    if args.discussion_category.is_some() {
        eyre::bail!(NO_DISCUSSIONS);
    }
    let notes = read_notes(&args.notes, &args.notes_file)?;
    let rel = fetch_release(&api, owner, repo, Some(&args.release)).await?;
    let id = rel
        .id
        .ok_or_else(|| eyre::eyre!("release {} has no id", args.release))?;

    let draft = args.draft.unwrap_or(rel.draft.unwrap_or(false));
    let prerelease = args.prerelease.unwrap_or(rel.prerelease.unwrap_or(false));
    if args.latest == Some(true) && (draft || prerelease) {
        eyre::bail!(
            "--latest can't be used with a draft or prerelease: Gitea only treats published, non-prerelease releases as latest"
        );
    }

    if args.verify_tag {
        let tag = args.new_tag.as_deref().unwrap_or(&args.release);
        verify_tag(&api, owner, repo, tag).await?;
    }

    let rel = api
        .repo_edit_release()
        .owner(owner)
        .repo(repo)
        .id(id)
        .body_map(|mut b| {
            if let Some(title) = &args.title {
                b = b.name(title.clone());
            }
            if let Some(notes) = &notes {
                b = b.body(notes.clone());
            }
            if let Some(draft) = args.draft {
                b = b.draft(draft);
            }
            if let Some(prerelease) = args.prerelease {
                b = b.prerelease(prerelease);
            }
            if let Some(tag) = &args.new_tag {
                b = b.tag_name(tag.clone());
            }
            if let Some(target) = &args.target {
                b = b.target_commitish(target.clone());
            }
            b
        })
        .send()
        .await
        .map_err(api_error)?
        .into_inner();

    println!("{}", rel.html_url.as_deref().unwrap_or(""));

    if let Some(want) = args.latest {
        check_latest(&api, owner, repo, &rel, want).await?;
    }
    Ok(())
}

async fn upload_assets(repo_args: &repo::RepoArgs, args: &UploadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let files = asset_paths(&args.files)?;
    let rel = fetch_release(&api, owner, repo, Some(&args.tag)).await?;
    let id = rel
        .id
        .ok_or_else(|| eyre::eyre!("release {} has no id", args.tag))?;

    // Assets each file would replace, by index into `args.files`.
    let mut existing = Vec::new();
    for file in &files {
        let name = file_name(file)?;
        let asset = rel
            .assets
            .iter()
            .find(|a| a.name.as_deref() == Some(name.as_str()));
        existing.push(asset.map(|a| (name, a.id)));
    }
    if !args.clobber {
        let dupes: Vec<&str> = existing.iter().flatten().map(|(n, _)| n.as_str()).collect();
        if !dupes.is_empty() {
            eyre::bail!(
                "asset under the same name already exists: [{}]",
                dupes.join(" ")
            );
        }
    }

    for (file, existing) in files.iter().zip(existing) {
        if let Some((_, Some(asset_id))) = existing {
            api.repo_delete_release_attachment()
                .owner(owner)
                .repo(repo)
                .id(id)
                .attachment_id(asset_id)
                .send()
                .await
                .map_err(api_error)?;
        }
        upload_asset(&api, &config, owner, repo, id, file).await?;
    }

    if atty_check() {
        eprintln!(
            "Successfully uploaded {} assets to {}",
            args.files.len(),
            args.tag
        );
    }
    Ok(())
}

async fn delete_asset(repo_args: &repo::RepoArgs, args: &DeleteAssetArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let rel = fetch_release(&api, owner, repo, Some(&args.tag)).await?;
    let id = rel
        .id
        .ok_or_else(|| eyre::eyre!("release {} has no id", args.tag))?;
    let asset_id = rel
        .assets
        .iter()
        .find(|a| a.name.as_deref() == Some(args.asset_name.as_str()))
        .and_then(|a| a.id)
        .ok_or_else(|| {
            eyre::eyre!(
                "asset {} not found in release {}",
                args.asset_name,
                args.tag
            )
        })?;

    if !args.yes && can_prompt() {
        confirm(&format!(
            "Delete asset {} in release {} in {owner}/{repo}?",
            args.asset_name, args.tag
        ))?;
    }

    api.repo_delete_release_attachment()
        .owner(owner)
        .repo(repo)
        .id(id)
        .attachment_id(asset_id)
        .send()
        .await
        .map_err(api_error)?;

    if atty_check() {
        eprintln!(
            "✓ Deleted asset {} from release {}",
            args.asset_name, args.tag
        );
    }
    Ok(())
}
