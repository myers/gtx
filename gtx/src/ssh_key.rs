use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use gitea_api::types::PublicKey;

#[derive(Args)]
pub struct SshKeyCommand {
    #[command(subcommand)]
    action: SshKeyAction,
}

#[derive(Subcommand)]
enum SshKeyAction {
    /// List your SSH keys
    List(ListArgs),
    /// Add an SSH key
    Add(AddArgs),
    /// Delete an SSH key
    Delete(DeleteArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct AddArgs {
    /// Key title
    #[arg(short, long)]
    title: String,

    /// SSH public key (or path to .pub file)
    key: String,
}

#[derive(Args)]
struct DeleteArgs {
    /// Key ID
    id: i64,
}

impl SshKeyCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            SshKeyAction::List(args) => list_keys(args).await,
            SshKeyAction::Add(args) => add_key(args).await,
            SshKeyAction::Delete(args) => delete_key(args).await,
        }
    }
}

/// `ssh-key list --json` fields (gh's `ssh-key list` has no `--json`;
/// camelCase like the rest).
const KEY_FIELDS: &[Field<PublicKey>] = &[
    field("createdAt", |k| gh::time(k.created_at)),
    field("fingerprint", |k| gh::v(&k.fingerprint)),
    field("id", |k| gh::v(k.id)),
    field("key", |k| gh::v(&k.key)),
    field("keyType", |k| gh::v(&k.key_type)),
    field("lastUsedAt", |k| gh::time(k.last_used_at)),
    field("readOnly", |k| gh::v(k.read_only.unwrap_or(false))),
    field("title", |k| gh::v(&k.title)),
    field("url", |k| gh::v(&k.url)),
];

async fn list_keys(args: &ListArgs) -> Result<()> {
    let json = args.json.select(KEY_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;

    let keys = api
        .user_current_list_keys()
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if let Some(json) = json {
        return json.write_list(&keys);
    }

    if keys.is_empty() {
        eprintln!("No SSH keys found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<6} {:<25} {:<12} {}", "ID", "TITLE", "TYPE", "ADDED");
    }

    for key in &keys {
        let id = key.id.unwrap_or(0);
        let title = key.title.as_deref().unwrap_or("");
        let key_type = key.key_type.as_deref().unwrap_or("");
        let created = key
            .created_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!("{:<6} {:<25} {:<12} {}", id, title, key_type, created);
    }

    Ok(())
}

async fn add_key(args: &AddArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let key_content = if std::path::Path::new(&args.key).exists() {
        std::fs::read_to_string(&args.key)?
    } else {
        args.key.clone()
    };

    let key = api
        .user_current_post_key()
        .body_map(|b| {
            b.title(args.title.clone())
                .key(key_content.trim().to_string())
        })
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = key.id.unwrap_or(0);
    let fingerprint = key.fingerprint.as_deref().unwrap_or("");
    eprintln!(
        "Added SSH key '{}' (ID: {id}, fingerprint: {fingerprint})",
        args.title
    );
    Ok(())
}

async fn delete_key(args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    api.user_current_delete_key()
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted SSH key #{}", args.id);
    Ok(())
}
