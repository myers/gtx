//! `gtx package`: list, view and delete an owner's packages. gh has no
//! package command, so this follows its owner-scoped commands' idioms
//! (`-o/--owner`, `-L/--limit`, `--json`, `-w/--web`, `--yes`). Publishing
//! stays with package tooling (cargo, docker, curl, ...).

use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use crate::paginate;
use crate::repo;
use gitea_api::types::{ListPackagesType, Package, PackageFile};

/// Package types Gitea's `GET /packages/{owner}?type=` accepts.
const PACKAGE_TYPES: [&str; 21] = [
    "alpine",
    "cargo",
    "chef",
    "composer",
    "conan",
    "conda",
    "container",
    "cran",
    "debian",
    "generic",
    "go",
    "helm",
    "maven",
    "npm",
    "nuget",
    "pub",
    "pypi",
    "rpm",
    "rubygems",
    "swift",
    "vagrant",
];

#[derive(Args)]
pub struct PackageCommand {
    /// Owner (user or organization) of the packages. Defaults to the owner
    /// of the current repository.
    #[arg(short = 'o', long, global = true)]
    owner: Option<String>,

    #[command(subcommand)]
    action: PackageAction,
}

#[derive(Subcommand)]
enum PackageAction {
    /// List package versions of an owner
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// View a package version and its files
    View(ViewArgs),
    /// Delete a package version
    Delete(DeleteArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Only list packages of this type
    #[arg(long = "type", value_parser = PACKAGE_TYPES)]
    type_: Option<String>,

    /// Only list packages whose name matches this query
    #[arg(short = 'S', long)]
    search: Option<String>,

    /// Maximum number of package versions to fetch
    #[arg(short = 'L', long, default_value_t = 30)]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    /// Package as TYPE/NAME (e.g. generic/mytool)
    package: String,

    /// Version; the latest when omitted
    version: Option<String>,

    /// Open the package in the browser
    #[arg(short, long)]
    web: bool,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct DeleteArgs {
    /// Package as TYPE/NAME (e.g. generic/mytool)
    package: String,

    /// Version to delete
    version: String,

    /// Confirm deletion without prompting
    #[arg(long)]
    yes: bool,
}

impl PackageCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            PackageAction::List(args) => list(self.owner.as_deref(), args).await,
            PackageAction::View(args) => view(self.owner.as_deref(), args).await,
            PackageAction::Delete(args) => delete(self.owner.as_deref(), args).await,
        }
    }
}

/// A package version plus its files (only fetched by `view`).
struct Row {
    pkg: Package,
    files: Vec<PackageFile>,
}

fn login(u: Option<&gitea_api::types::User>) -> serde_json::Value {
    u.map_or(
        serde_json::Value::Null,
        |u| serde_json::json!({"login": u.login}),
    )
}

const LIST_FIELDS: &[Field<Row>] = &[
    field("createdAt", |r| gh::time(r.pkg.created_at)),
    field("creator", |r| login(r.pkg.creator.as_ref())),
    field("id", |r| gh::v(r.pkg.id)),
    field("name", |r| gh::v(&r.pkg.name)),
    field("owner", |r| login(r.pkg.owner.as_ref())),
    field("repository", |r| {
        r.pkg
            .repository
            .as_ref()
            .map_or(serde_json::Value::Null, |repo| {
                gh::repo_ref(repo.name.as_deref(), repo.full_name.as_deref())
            })
    }),
    field("type", |r| gh::v(&r.pkg.type_)),
    field("url", |r| gh::v(&r.pkg.html_url)),
    field("version", |r| gh::v(&r.pkg.version)),
];

const VIEW_FIELDS: &[Field<Row>] = &[
    field("createdAt", |r| gh::time(r.pkg.created_at)),
    field("creator", |r| login(r.pkg.creator.as_ref())),
    field("files", |r| {
        serde_json::Value::Array(
            r.files
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "id": f.id, "name": f.name, "size": f.size,
                        "md5": f.md5, "sha1": f.sha1, "sha256": f.sha256, "sha512": f.sha512,
                    })
                })
                .collect(),
        )
    }),
    field("id", |r| gh::v(r.pkg.id)),
    field("name", |r| gh::v(&r.pkg.name)),
    field("owner", |r| login(r.pkg.owner.as_ref())),
    field("repository", |r| {
        r.pkg
            .repository
            .as_ref()
            .map_or(serde_json::Value::Null, |repo| {
                gh::repo_ref(repo.name.as_deref(), repo.full_name.as_deref())
            })
    }),
    field("type", |r| gh::v(&r.pkg.type_)),
    field("url", |r| gh::v(&r.pkg.html_url)),
    field("version", |r| gh::v(&r.pkg.version)),
];

fn api_error(e: impl Into<gitea_api::GiteaError>) -> eyre::Report {
    eyre::eyre!("{}", e.into())
}

/// `-o/--owner`, else the current repository's owner.
fn resolve_owner(explicit: Option<&str>, config: &Config) -> Result<String> {
    if let Some(o) = explicit {
        return Ok(o.to_string());
    }
    repo::resolve_repo(None, &config.url)
        .map(|r| r.owner)
        .map_err(|e| eyre::eyre!("{e}\nOr use -o/--owner to name the package owner."))
}

/// Split `TYPE/NAME`. Names may themselves contain `/` (container images).
fn parse_package(s: &str) -> Result<(&str, &str)> {
    match s.split_once('/') {
        Some((t, n)) if !t.is_empty() && !n.is_empty() => Ok((t, n)),
        _ => eyre::bail!("Package must be given as TYPE/NAME (e.g. generic/mytool), got: {s}"),
    }
}

fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

async fn list(owner: Option<&str>, args: &ListArgs) -> Result<()> {
    let json = args.json.select(LIST_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let owner = resolve_owner(owner, &config)?;
    let type_: Option<ListPackagesType> = args
        .type_
        .as_deref()
        .map(|t| t.parse())
        .transpose()
        .map_err(|e| eyre::eyre!("{e}"))?;

    let packages = paginate::paginate(args.limit, 50, |page, per_page| {
        let (api, owner, type_) = (&api, owner.as_str(), type_);
        let search = args.search.clone();
        async move {
            let mut req = api.list_packages().owner(owner).page(page).limit(per_page);
            if let Some(t) = type_ {
                req = req.type_(t);
            }
            if let Some(q) = search {
                req = req.q(q);
            }
            Ok(req.send().await.map_err(api_error)?.into_inner())
        }
    })
    .await?;
    let rows: Vec<Row> = packages
        .into_iter()
        .map(|pkg| Row {
            pkg,
            files: Vec::new(),
        })
        .collect();

    if let Some(json) = json {
        return json.write_list(&rows);
    }

    let is_tty = atty_check();
    if rows.is_empty() {
        if is_tty {
            eprintln!("no packages found for {owner}");
        }
        return Ok(());
    }

    let table: Vec<[String; 5]> = rows
        .iter()
        .map(|Row { pkg, .. }| {
            let created = match pkg.created_at {
                Some(dt) if is_tty => relative_time(dt),
                Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                None => String::new(),
            };
            let repo = pkg
                .repository
                .as_ref()
                .and_then(|r| r.full_name.clone())
                .unwrap_or_default();
            [
                pkg.name.clone().unwrap_or_default(),
                pkg.type_.clone().unwrap_or_default(),
                pkg.version.clone().unwrap_or_default(),
                repo,
                created,
            ]
        })
        .collect();

    if !is_tty {
        for row in &table {
            println!("{}", row.join("\t"));
        }
        return Ok(());
    }

    let header = ["NAME", "TYPE", "VERSION", "REPOSITORY", "CREATED"];
    let widths: Vec<usize> = (0..4)
        .map(|i| {
            table
                .iter()
                .map(|r| r[i].chars().count())
                .chain([header[i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cols: [&str; 5]| {
        format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
            cols[0],
            cols[1],
            cols[2],
            cols[3],
            cols[4],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
        )
    };
    println!("{}", line(header).trim_end());
    for r in &table {
        println!(
            "{}",
            line([&r[0], &r[1], &r[2], &r[3], &r[4]].map(String::as_str)).trim_end()
        );
    }
    Ok(())
}

async fn view(owner: Option<&str>, args: &ViewArgs) -> Result<()> {
    let json = args.json.select(VIEW_FIELDS)?;
    let (type_, name) = parse_package(&args.package)?;
    let config = Config::load()?;
    let api = config.client()?;
    let owner = resolve_owner(owner, &config)?;

    let pkg = match &args.version {
        Some(version) => api
            .get_package()
            .owner(&owner)
            .type_(type_)
            .name(name)
            .version(version)
            .send()
            .await
            .map_err(api_error)?
            .into_inner(),
        None => api
            .get_latest_package_version()
            .owner(&owner)
            .type_(type_)
            .name(name)
            .send()
            .await
            .map_err(api_error)?
            .into_inner(),
    };

    if args.web {
        let url = pkg.html_url.as_deref().unwrap_or("");
        if atty_check() {
            eprintln!("Opening {url} in your browser.");
        }
        return crate::browse::open_url(url);
    }

    let version = pkg
        .version
        .clone()
        .or_else(|| args.version.clone())
        .unwrap_or_default();
    let files = if json.as_ref().is_none_or(|j| j.wants("files")) {
        api.list_package_files()
            .owner(&owner)
            .type_(type_)
            .name(name)
            .version(&version)
            .send()
            .await
            .map_err(api_error)?
            .into_inner()
    } else {
        Vec::new()
    };
    let row = Row { pkg, files };

    if let Some(json) = json {
        return json.write_one(&row);
    }

    let Row { pkg, files } = &row;
    let repo = pkg.repository.as_ref().and_then(|r| r.full_name.as_deref());
    let creator = pkg.creator.as_ref().and_then(|u| u.login.as_deref());
    let url = pkg.html_url.as_deref().unwrap_or("");
    if atty_check() {
        println!("{type_}/{name} {version}");
        let mut meta = vec![format!("Owner: {owner}")];
        if let Some(repo) = repo {
            meta.push(format!("Repository: {repo}"));
        }
        println!("{}", meta.join(" • "));
        if let Some(dt) = pkg.created_at {
            println!(
                "Published by {} {}",
                creator.unwrap_or("unknown"),
                relative_time(dt)
            );
        }
        println!();
        if files.is_empty() {
            println!("No files");
        } else {
            println!("Files");
            let width = files
                .iter()
                .map(|f| f.name.as_deref().unwrap_or("").chars().count())
                .max()
                .unwrap_or(0);
            for f in files {
                println!(
                    "  {:<width$}  {:>10}  sha256:{}",
                    f.name.as_deref().unwrap_or(""),
                    format_size(f.size.unwrap_or(0)),
                    f.sha256.as_deref().unwrap_or(""),
                );
            }
        }
        println!();
        println!("View this package on Gitea: {url}");
    } else {
        println!("name:\t{name}");
        println!("type:\t{type_}");
        println!("version:\t{version}");
        println!("owner:\t{owner}");
        println!("repository:\t{}", repo.unwrap_or(""));
        println!("creator:\t{}", creator.unwrap_or(""));
        if let Some(dt) = pkg.created_at {
            println!(
                "created:\t{}",
                dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            );
        }
        println!("url:\t{url}");
        for f in files {
            println!(
                "file:\t{}\t{}\t{}",
                f.name.as_deref().unwrap_or(""),
                f.size.unwrap_or(0),
                f.sha256.as_deref().unwrap_or(""),
            );
        }
    }
    Ok(())
}

async fn delete(owner: Option<&str>, args: &DeleteArgs) -> Result<()> {
    let (type_, name) = parse_package(&args.package)?;
    let config = Config::load()?;
    let api = config.client()?;
    let owner = resolve_owner(owner, &config)?;

    let what = format!("{type_}/{name} {} of {owner}", args.version);
    crate::prompt::confirm_deletion(args.yes, &what, name)?;

    api.delete_package()
        .owner(&owner)
        .type_(type_)
        .name(name)
        .version(&args.version)
        .send()
        .await
        .map_err(api_error)?;

    eprintln!("Deleted package {type_}/{name} {} of {owner}", args.version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_types_all_parse() {
        for t in PACKAGE_TYPES {
            assert!(t.parse::<ListPackagesType>().is_ok(), "{t}");
        }
    }

    #[test]
    fn parse_package_keeps_slashes_in_name() {
        assert_eq!(
            parse_package("container/org/img").unwrap(),
            ("container", "org/img")
        );
        assert!(parse_package("generic").is_err());
        assert!(parse_package("/x").is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(12), "12 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
