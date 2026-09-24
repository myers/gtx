use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::json::{Field, field, gh};
use gitea_api::types::Organization;
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

    #[command(flatten)]
    json: crate::json::JsonArgs,
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

/// `org list/view --json` fields (gtx-only commands; camelCase like the rest,
/// with gh's `login` for the org's username and `name` for its full name).
const ORG_FIELDS: &[Field<Organization>] = &[
    field("avatarUrl", |o| gh::v(&o.avatar_url)),
    field("description", |o| gh::v(o.description.as_deref().unwrap_or(""))),
    field("email", |o| gh::v(o.email.as_deref().unwrap_or(""))),
    field("id", |o| gh::v(o.id)),
    field("location", |o| gh::v(o.location.as_deref().unwrap_or(""))),
    field("login", |o| gh::v(&o.username)),
    field("name", |o| gh::v(o.full_name.as_deref().unwrap_or(""))),
    field("visibility", |o| gh::v(&o.visibility)),
    field("website", |o| gh::v(o.website.as_deref().unwrap_or(""))),
];

async fn list_orgs(args: &ListArgs) -> Result<()> {
    let json = args.json.select(ORG_FIELDS)?;
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

    if let Some(json) = json {
        return json.write_list(&orgs);
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
    let json = args.json.select(ORG_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;

    let org = api
        .org_get()
        .org(&args.name)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(json) = json {
        return json.write_one(&org);
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
