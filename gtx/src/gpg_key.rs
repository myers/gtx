use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};

#[derive(Args)]
pub struct GpgKeyCommand {
    #[command(subcommand)]
    action: GpgKeyAction,
}

#[derive(Subcommand)]
enum GpgKeyAction {
    /// List your GPG keys
    List(ListArgs),
    /// Add a GPG key
    Add(AddArgs),
    /// Delete a GPG key
    Delete(DeleteArgs),
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct AddArgs {
    /// Armored GPG public key (or path to file)
    key: String,
}

#[derive(Args)]
struct DeleteArgs {
    /// Key ID
    id: i64,
}

impl GpgKeyCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            GpgKeyAction::List(args) => list_keys(args).await,
            GpgKeyAction::Add(args) => add_key(args).await,
            GpgKeyAction::Delete(args) => delete_key(args).await,
        }
    }
}

const KEY_FIELDS: &[&str] = &[
    "id", "key_id", "emails", "can_sign", "can_certify",
    "can_encrypt_comms", "can_encrypt_storage", "expires_at", "created_at",
];

async fn list_keys(args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let keys = api
        .user_current_list_gpg_keys()
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &keys, KEY_FIELDS);
    }

    if keys.is_empty() {
        eprintln!("No GPG keys found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<18} {:<30} {:<6} {}",
            "ID", "KEY ID", "EMAILS", "SIGN", "ADDED"
        );
    }

    for key in &keys {
        let id = key.id.unwrap_or(0);
        let key_id = key.key_id.as_deref().unwrap_or("");
        let emails: Vec<&str> = key
            .emails
            .iter()
            .filter_map(|e| e.email.as_deref())
            .collect();
        let emails_str = emails.join(", ");
        let truncated_emails = if emails_str.len() > 28 {
            format!("{}...", &emails_str[..25])
        } else {
            emails_str
        };
        let can_sign = if key.can_sign.unwrap_or(false) {
            "✓"
        } else {
            ""
        };
        let created = key
            .created_at
            .map(|dt| relative_time(dt))
            .unwrap_or_default();

        println!(
            "{:<6} {:<18} {:<30} {:<6} {}",
            id, key_id, truncated_emails, can_sign, created
        );
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
        .user_current_post_gpg_key()
        .body_map(|b| b.armored_public_key(key_content.trim().to_string()))
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let id = key.id.unwrap_or(0);
    let key_id = key.key_id.as_deref().unwrap_or("");
    eprintln!("Added GPG key {key_id} (ID: {id})");
    Ok(())
}

async fn delete_key(args: &DeleteArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    api.user_current_delete_gpg_key()
        .id(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Deleted GPG key #{}", args.id);
    Ok(())
}
