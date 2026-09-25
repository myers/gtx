//! Labels, milestones and assignees on issues and PRs: gh's `--add-label` /
//! `--remove-label` / `--milestone` / `--remove-milestone` / `--add-assignee` /
//! `--remove-assignee` edit flags, plus the name → ID lookups `issue create`
//! and `issue list` share. Gitea addresses labels and milestones by ID, and a
//! PR is an issue, so everything goes through the issue endpoints.

use clap::Args;
use eyre::Result;
use gitea_api::types::{Issue, Label, Milestone};

use crate::paginate;

fn api_err(e: impl Into<gitea_api::GiteaError>) -> eyre::Report {
    eyre::eyre!("{}", e.into())
}

/// Every label of `owner/repo`.
async fn repo_labels(api: &gitea_api::Gitea, owner: &str, repo: &str) -> Result<Vec<Label>> {
    paginate::paginate(i64::MAX, 50, |page, per_page| async move {
        Ok(api
            .issue_list_labels()
            .owner(owner)
            .repo(repo)
            .page(page)
            .limit(per_page)
            .send()
            .await
            .map_err(api_err)?
            .into_inner())
    })
    .await
}

/// IDs of the labels named `names`, or `Ok(Err(name))` for the first name
/// the repo has no label for.
async fn try_label_ids(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    names: &[String],
) -> Result<std::result::Result<Vec<i64>, String>> {
    if names.is_empty() {
        return Ok(Ok(Vec::new()));
    }
    let labels = repo_labels(api, owner, repo).await?;
    let mut ids = Vec::new();
    for name in names {
        match labels.iter().find(|l| l.name.as_deref() == Some(name)) {
            Some(l) => ids.push(l.id.unwrap_or(0)),
            None => return Ok(Err(name.clone())),
        }
    }
    Ok(Ok(ids))
}

/// IDs of the labels named `names`; an unknown name errors like gh.
pub async fn label_ids(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    names: &[String],
) -> Result<Vec<i64>> {
    try_label_ids(api, owner, repo, names)
        .await?
        .map_err(|name| eyre::eyre!("could not find label: '{name}' not found"))
}

/// Like [`label_ids`], but `None` when some label doesn't exist — for list
/// filters, where gh then just matches nothing.
pub async fn filter_label_ids(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    names: &[String],
) -> Result<Option<Vec<i64>>> {
    Ok(try_label_ids(api, owner, repo, names).await?.ok())
}

/// Every milestone of `owner/repo`, open or closed.
async fn repo_milestones(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
) -> Result<Vec<Milestone>> {
    paginate::paginate(i64::MAX, 50, |page, per_page| async move {
        Ok(api
            .issue_get_milestones_list()
            .owner(owner)
            .repo(repo)
            .state("all")
            .page(page)
            .limit(per_page)
            .send()
            .await
            .map_err(api_err)?
            .into_inner())
    })
    .await
}

/// ID of the milestone titled `title` (case-insensitive, as gh matches).
pub async fn find_milestone_id(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    title: &str,
) -> Result<Option<i64>> {
    let milestones = repo_milestones(api, owner, repo).await?;
    Ok(milestones
        .iter()
        .find(|m| {
            m.title
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(title))
        })
        .map(|m| m.id.unwrap_or(0)))
}

/// Like [`find_milestone_id`], but errors like gh when there is none.
pub async fn milestone_id(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    title: &str,
) -> Result<i64> {
    find_milestone_id(api, owner, repo, title)
        .await?
        .ok_or_else(|| eyre::eyre!("could not add to milestone '{title}': '{title}' not found"))
}

/// `logins` with gh's `@me` replaced by the authenticated user's login.
pub async fn resolve_logins(api: &gitea_api::Gitea, logins: &[String]) -> Result<Vec<String>> {
    let mut me = None;
    let mut out = Vec::with_capacity(logins.len());
    for login in logins {
        if login == "@me" {
            if me.is_none() {
                let user = api
                    .user_get_current()
                    .send()
                    .await
                    .map_err(api_err)?
                    .into_inner();
                me = Some(user.login.unwrap_or_default());
            }
            out.push(me.clone().unwrap_or_default());
        } else {
            out.push(login.clone());
        }
    }
    Ok(out)
}

/// gh's metadata edit flags, shared by `issue edit` and `pr edit`.
#[derive(Args, Default)]
pub struct EditMeta {
    /// Add labels by name
    #[arg(long, value_name = "name", value_delimiter = ',')]
    pub add_label: Vec<String>,

    /// Remove labels by name
    #[arg(long, value_name = "name", value_delimiter = ',')]
    pub remove_label: Vec<String>,

    /// Add assigned users by their login. Use "@me" to assign yourself.
    #[arg(long, value_name = "login", value_delimiter = ',')]
    pub add_assignee: Vec<String>,

    /// Remove assigned users by their login. Use "@me" to unassign yourself.
    #[arg(long, value_name = "login", value_delimiter = ',')]
    pub remove_assignee: Vec<String>,

    /// Edit the milestone by name
    #[arg(short, long, value_name = "name", conflicts_with = "remove_milestone")]
    pub milestone: Option<String>,

    /// Remove the milestone association
    #[arg(long)]
    pub remove_milestone: bool,
}

impl EditMeta {
    pub fn is_empty(&self) -> bool {
        self.add_label.is_empty()
            && self.remove_label.is_empty()
            && self.add_assignee.is_empty()
            && self.remove_assignee.is_empty()
            && self.milestone.is_none()
            && !self.remove_milestone
    }
}

/// Apply `title`, `body` and `meta` to issue (or PR) `number`, returning the
/// edited issue.
pub async fn edit(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    number: i64,
    title: Option<&str>,
    body: Option<&str>,
    meta: &EditMeta,
) -> Result<Issue> {
    // Resolve every name before changing anything, so a typo edits nothing.
    let add_ids = label_ids(api, owner, repo, &meta.add_label).await?;
    let remove_ids = label_ids(api, owner, repo, &meta.remove_label).await?;
    let milestone = match (&meta.milestone, meta.remove_milestone) {
        (Some(title), _) => Some(milestone_id(api, owner, repo, title).await?),
        (None, true) => Some(0),
        (None, false) => None,
    };
    let assignees = if meta.add_assignee.is_empty() && meta.remove_assignee.is_empty() {
        None
    } else {
        let add = resolve_logins(api, &meta.add_assignee).await?;
        let remove = resolve_logins(api, &meta.remove_assignee).await?;
        let issue = api
            .issue_get_issue()
            .owner(owner)
            .repo(repo)
            .index(number)
            .send()
            .await
            .map_err(api_err)?
            .into_inner();
        let mut logins: Vec<String> = issue
            .assignees
            .iter()
            .filter_map(|u| u.login.clone())
            .collect();
        for login in add {
            if !logins.contains(&login) {
                logins.push(login);
            }
        }
        logins.retain(|l| !remove.contains(l));
        Some(logins)
    };

    if !add_ids.is_empty() {
        api.issue_add_label()
            .owner(owner)
            .repo(repo)
            .index(number)
            .body_map(|b| b.labels(add_ids.iter().map(|&id| id.into()).collect::<Vec<_>>()))
            .send()
            .await
            .map_err(api_err)?;
    }
    for id in remove_ids {
        api.issue_remove_label()
            .owner(owner)
            .repo(repo)
            .index(number)
            .id(id)
            .send()
            .await
            .map_err(api_err)?;
    }

    let issue = api
        .issue_edit_issue()
        .owner(owner)
        .repo(repo)
        .index(number)
        .body_map(|mut b| {
            if let Some(t) = title {
                b = b.title(t.to_string());
            }
            if let Some(bd) = body {
                b = b.body(bd.to_string());
            }
            if let Some(ms) = milestone {
                b = b.milestone(ms);
            }
            match assignees {
                // An empty `assignees` isn't sent; Gitea treats a lone empty
                // `assignee` as "unassign everyone".
                Some(logins) if logins.is_empty() => b = b.assignee(String::new()),
                Some(logins) => b = b.assignees(logins),
                None => {}
            }
            b
        })
        .send()
        .await
        .map_err(api_err)?
        .into_inner();
    Ok(issue)
}
