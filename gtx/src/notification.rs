use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};

#[derive(Args)]
pub struct NotificationCommand {
    #[command(subcommand)]
    action: NotificationAction,
}

#[derive(Subcommand)]
enum NotificationAction {
    /// List notifications
    List(ListArgs),
    /// Mark notifications as read
    Read(ReadArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Show only unread notifications
    #[arg(short, long)]
    unread: bool,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ReadArgs {
    /// Notification thread ID (omit to mark all as read)
    id: Option<String>,
}

impl NotificationCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            NotificationAction::List(args) => list_notifications(args).await,
            NotificationAction::Read(args) => read_notifications(args).await,
        }
    }
}

const NOTIFICATION_FIELDS: &[&str] = &[
    "id", "subject", "repository", "unread", "pinned", "updated_at", "url",
];

async fn list_notifications(args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    let path = if args.unread {
        "notifications?status-types=unread"
    } else {
        "notifications"
    };

    let resp = api.raw_get(path).await.map_err(|e| eyre::eyre!("{e}"))?;
    let notifications: Vec<serde_json::Value> = serde_json::from_str(&resp)?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &notifications, NOTIFICATION_FIELDS);
    }

    if notifications.is_empty() {
        eprintln!("No notifications");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<6} {:<8} {:<25} {:<40} {}",
            "ID", "TYPE", "REPO", "SUBJECT", "UPDATED"
        );
    }

    for n in &notifications {
        let id = n["id"].as_i64().unwrap_or(0);
        let unread = if n["unread"].as_bool().unwrap_or(false) {
            "●"
        } else {
            " "
        };

        let repo_name = n["repository"]["full_name"]
            .as_str()
            .unwrap_or("");
        let truncated_repo = if repo_name.len() > 23 {
            format!("{}...", &repo_name[..20])
        } else {
            repo_name.to_string()
        };

        let subject_type = n["subject"]["type"].as_str().unwrap_or("");
        let subject_title = n["subject"]["title"].as_str().unwrap_or("");
        let truncated_title = if subject_title.len() > 38 {
            format!("{}...", &subject_title[..35])
        } else {
            subject_title.to_string()
        };

        let state = n["subject"]["state"].as_str().unwrap_or("");
        let state_indicator = match state {
            "open" => "",
            "closed" => " ✓",
            "merged" => " ⊕",
            _ => "",
        };

        let updated = n["updated_at"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| relative_time(dt.with_timezone(&chrono::Utc)))
            .unwrap_or_default();

        println!(
            "{:<6} {unread}{:<7} {:<25} {:<40} {}",
            id, subject_type, truncated_repo, format!("{truncated_title}{state_indicator}"), updated
        );
    }

    Ok(())
}

async fn read_notifications(args: &ReadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;

    if let Some(ref id) = args.id {
        // Mark specific thread as read
        api.notify_read_thread()
            .id(id)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
        eprintln!("Notification #{id} marked as read");
    } else {
        // Mark all as read
        api.notify_read_list()
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;
        eprintln!("All notifications marked as read");
    }

    Ok(())
}
