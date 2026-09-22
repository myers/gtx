use clap::Args;
use eyre::Result;

use crate::config::Config;

#[derive(Args)]
pub struct StatusCommand {}

impl StatusCommand {
    pub async fn run(&self) -> Result<()> {
        let config = Config::load()?;
        let api = config.client()?;

        // Fetch all three sections in parallel
        let (notifications, assigned, review_requested) = tokio::join!(
            fetch_notifications(&api),
            fetch_assigned(&api),
            fetch_review_requests(&api),
        );

        let notifications = notifications?;
        let assigned = assigned?;
        let review_requested = review_requested?;

        // Notifications
        if notifications.is_empty() {
            println!("Notifications: none");
        } else {
            println!("Notifications ({} unread)", notifications.len());
            for n in &notifications {
                let repo = n["repository"]["full_name"].as_str().unwrap_or("");
                let kind = n["subject"]["type"].as_str().unwrap_or("");
                let title = n["subject"]["title"].as_str().unwrap_or("");
                let state = n["subject"]["state"].as_str().unwrap_or("");
                let icon = match state {
                    "closed" => " ✓",
                    "merged" => " ⊕",
                    _ => "",
                };
                let truncated = if title.len() > 45 {
                    format!("{}...", &title[..42])
                } else {
                    title.to_string()
                };
                println!("  ● {:<7} {:<25} {}{}", kind, repo, truncated, icon);
            }
        }

        println!();

        // Assigned issues
        if assigned.is_empty() {
            println!("Assigned Issues: none");
        } else {
            println!("Assigned Issues ({})", assigned.len());
            for issue in &assigned {
                let repo = issue
                    .repository
                    .as_ref()
                    .and_then(|r| r.full_name.as_deref())
                    .unwrap_or("");
                let number = issue.number.unwrap_or(0);
                let title = issue.title.as_deref().unwrap_or("");
                let truncated = if title.len() > 45 {
                    format!("{}...", &title[..42])
                } else {
                    title.to_string()
                };
                println!("  {repo}#{number:<6} {truncated}");
            }
        }

        println!();

        // Review requests
        if review_requested.is_empty() {
            println!("Review Requests: none");
        } else {
            println!("Review Requests ({})", review_requested.len());
            for pr in &review_requested {
                let repo = pr
                    .repository
                    .as_ref()
                    .and_then(|r| r.full_name.as_deref())
                    .unwrap_or("");
                let number = pr.number.unwrap_or(0);
                let title = pr.title.as_deref().unwrap_or("");
                let truncated = if title.len() > 45 {
                    format!("{}...", &title[..42])
                } else {
                    title.to_string()
                };
                println!("  {repo}#{number:<6} {truncated}");
            }
        }

        Ok(())
    }
}

async fn fetch_notifications(api: &gitea_api::Gitea) -> Result<Vec<serde_json::Value>> {
    let resp = api
        .raw_get("notifications?status-types=unread&limit=10")
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    Ok(serde_json::from_str(&resp)?)
}

async fn fetch_assigned(
    api: &gitea_api::Gitea,
) -> Result<Vec<gitea_api::types::Issue>> {
    let issues = api
        .issue_search_issues()
        .assigned(true)
        .state(gitea_api::types::IssueSearchIssuesState::Open)
        .type_(gitea_api::types::IssueSearchIssuesType::Issues)
        .limit(10u64)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();
    Ok(issues)
}

async fn fetch_review_requests(
    api: &gitea_api::Gitea,
) -> Result<Vec<gitea_api::types::Issue>> {
    let prs = api
        .issue_search_issues()
        .review_requested(true)
        .state(gitea_api::types::IssueSearchIssuesState::Open)
        .type_(gitea_api::types::IssueSearchIssuesType::Pulls)
        .limit(10u64)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();
    Ok(prs)
}
