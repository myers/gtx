use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::atty_check;
use crate::paginate;

#[derive(Args)]
pub struct OrgCommand {
    #[command(subcommand)]
    action: OrgAction,
}

#[derive(Subcommand)]
enum OrgAction {
    /// List organizations
    List(ListArgs),
    /// View an organization
    View(ViewArgs),
    /// Create an organization
    Create(CreateArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// Organization username
    #[arg(short, long)]
    name: String,

    /// Description
    #[arg(short, long)]
    description: Option<String>,

    /// Visibility (public or private)
    #[arg(long, default_value = "public")]
    visibility: String,
}

#[derive(Args)]
struct ViewArgs {
    /// Organization name
    name: String,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

impl OrgCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            OrgAction::List(args) => list_orgs(args).await,
            OrgAction::View(args) => view_org(args).await,
            OrgAction::Create(args) => create_org(args).await,
        }
    }
}

const ORG_FIELDS: &[&str] = &[
    "id", "name", "full_name", "description", "website", "location",
    "avatar_url", "visibility",
];

async fn list_orgs(args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let orgs = paginate::paginate(200, 50, |page, per_page| {
        let api = &api;
        async move {
            Ok(api
                .org_list_current_user_orgs()
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
        return crate::json::write_json(&args.json, &orgs, &ORG_FIELDS);
    }

    if orgs.is_empty() {
        eprintln!("No organizations found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<30} {}", "ID", "NAME", "DESCRIPTION");
    }

    for org in &orgs {
        let id = org.id.unwrap_or(0);
        let name = org.username.as_deref().unwrap_or("");
        let desc = org.description.as_deref().unwrap_or("");
        let truncated_desc = if desc.len() > 50 {
            format!("{}...", &desc[..47])
        } else {
            desc.to_string()
        };
        println!("{:<6} {:<30} {}", id, name, truncated_desc);
    }

    Ok(())
}

async fn view_org(args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let org = api
        .org_get()
        .org(&args.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&org)?);
        return Ok(());
    }

    let name = org.username.as_deref().unwrap_or("(unknown)");
    let full_name = org.full_name.as_deref().unwrap_or("");
    let desc = org.description.as_deref().unwrap_or("");
    let visibility = org.visibility.as_deref().unwrap_or("");
    let location = org.location.as_deref().unwrap_or("");
    let website = org.website.as_deref().unwrap_or("");

    println!("{name}");
    if !full_name.is_empty() {
        println!("{full_name}");
    }
    println!("Visibility: {visibility}");

    if !desc.is_empty() {
        println!();
        println!("{desc}");
    }

    if !location.is_empty() {
        println!("Location: {location}");
    }
    if !website.is_empty() {
        println!("Website: {website}");
    }

    Ok(())
}

async fn create_org(args: &CreateArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let desc = args.description.clone();
    let vis = match args.visibility.as_str() {
        "public" => gitea_api::types::VisibilityEnum::Public,
        "limited" => gitea_api::types::VisibilityEnum::Limited,
        "private" => gitea_api::types::VisibilityEnum::Private,
        other => eyre::bail!("Invalid visibility: {other}. Use public, limited, or private"),
    };
    let org = api
        .org_create()
        .body_map(|mut b| {
            b = b.username(args.name.clone()).visibility(vis.clone());
            if let Some(d) = desc {
                b = b.description(d);
            }
            b
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let name = org.username.as_deref().unwrap_or("");
    eprintln!("Created organization: {name}");
    Ok(())
}
