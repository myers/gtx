use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::repo;

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "success" | "failure" | "cancelled" | "skipped" | "timed_out" | "action_required",
    )
}

fn is_failure_conclusion(conclusion: &str) -> bool {
    matches!(
        conclusion,
        "failure" | "cancelled" | "timed_out" | "action_required",
    )
}

fn job_icon(status: &str, conclusion: &str) -> &'static str {
    match (status, conclusion) {
        (_, "success") => "✓",
        (_, "failure") => "✗",
        (_, "cancelled") => "⊘",
        (_, "timed_out") => "✗",
        (_, "action_required") => "✗",
        ("in_progress", _) => "●",
        ("queued", _) | ("waiting", _) => "○",
        _ => "?",
    }
}

fn partition_runs_for_picker(
    runs: Vec<gitea_api::types::ActionWorkflowRun>,
) -> (
    Vec<gitea_api::types::ActionWorkflowRun>,
    Vec<gitea_api::types::ActionWorkflowRun>,
) {
    let mut in_progress = Vec::new();
    let mut recent = Vec::new();
    for run in runs {
        let status = run.status.as_deref().unwrap_or("");
        if matches!(status, "in_progress" | "queued" | "waiting") {
            in_progress.push(run);
        } else {
            recent.push(run);
        }
    }
    (in_progress, recent)
}

fn render_run_state(
    run: &gitea_api::types::ActionWorkflowRun,
    jobs: &[gitea_api::types::ActionWorkflowJob],
    compact: bool,
) -> String {
    let mut out = String::new();
    let id = run.id.unwrap_or(0);
    let title = run.display_title.as_deref().unwrap_or("(unnamed)");
    let status = run.status.as_deref().unwrap_or("unknown");
    let conclusion = run.conclusion.as_deref().unwrap_or("");

    out.push_str(&format!("Run #{id} — {title}\n"));
    if conclusion.is_empty() {
        out.push_str(&format!("Status: {status}\n"));
    } else {
        out.push_str(&format!("Status: {status} ({conclusion})\n"));
    }
    out.push('\n');

    let mut any_job_rendered = false;
    for job in jobs {
        let jname = job.name.as_deref().unwrap_or("(unnamed job)");
        let jstatus = job.status.as_deref().unwrap_or("");
        let jconclusion = job.conclusion.as_deref().unwrap_or("");

        if compact && job_is_fully_green(job) {
            continue;
        }
        any_job_rendered = true;

        let icon = job_icon(jstatus, jconclusion);
        if jconclusion.is_empty() {
            out.push_str(&format!("  {icon} {jname} ({jstatus})\n"));
        } else if jconclusion == "success" {
            out.push_str(&format!("  {icon} {jname}\n"));
        } else {
            out.push_str(&format!("  {icon} {jname} ({jconclusion})\n"));
        }

        for step in &job.steps {
            if compact && step_is_hidden_in_compact(step) {
                continue;
            }
            let sname = step.name.as_deref().unwrap_or("(unnamed step)");
            let sstatus = step.status.as_deref().unwrap_or("");
            let sconclusion = step.conclusion.as_deref().unwrap_or("");
            let sicon = job_icon(sstatus, sconclusion);
            out.push_str(&format!("    {sicon} {sname}\n"));
        }
    }

    if compact && !any_job_rendered {
        out.push_str("  (all steps passing so far)\n");
    }

    out
}

fn job_is_fully_green(job: &gitea_api::types::ActionWorkflowJob) -> bool {
    let conclusion = job.conclusion.as_deref().unwrap_or("");
    if conclusion != "success" {
        return false;
    }
    job.steps
        .iter()
        .all(|s| s.conclusion.as_deref() == Some("success"))
}

fn step_is_hidden_in_compact(step: &gitea_api::types::ActionWorkflowStep) -> bool {
    let conclusion = step.conclusion.as_deref().unwrap_or("");
    let status = step.status.as_deref().unwrap_or("");
    let visible = matches!(conclusion, "failure" | "cancelled" | "timed_out" | "action_required")
        || matches!(status, "in_progress" | "queued" | "waiting");
    !visible
}

fn redraw(prev_lines: &mut Option<usize>, body: &str, is_tty: bool) {
    let line_count = body.lines().count();
    if is_tty {
        if let Some(n) = prev_lines {
            eprint!("\x1b[{n}F\x1b[J");
        }
    } else if prev_lines.is_some() {
        eprintln!();
    }
    eprint!("{body}");
    *prev_lines = Some(line_count);
}

#[derive(Args)]
pub struct RunCommand {
    #[command(flatten)]
    pub repo: repo::RepoArgs,

    #[command(subcommand)]
    action: RunAction,
}

#[derive(Subcommand)]
enum RunAction {
    /// List workflow runs
    List(ListArgs),
    /// View a workflow run
    View(ViewArgs),
    /// Rerun a workflow run
    Rerun(RerunArgs),
    /// Watch a workflow run (poll until complete, show logs)
    Watch(WatchArgs),
    /// Download artifacts from a workflow run
    Download(DownloadArgs),
}

/// Run statuses Gitea's `status` query parameter accepts.
const RUN_STATUSES: &[&str] = &[
    "queued", "in_progress", "completed", "pending", "waiting", "requested",
    "action_required", "success", "failure", "skipped", "neutral", "cancelled",
    "timed_out",
];

/// Server-side filters for the workflow-run list endpoint.
#[derive(Args, Default)]
struct RunFilterArgs {
    /// Filter runs by the full SHA of the commit that triggered them
    #[arg(short = 'c', long, value_name = "SHA")]
    commit: Option<String>,

    /// Filter runs by branch
    #[arg(short, long)]
    branch: Option<String>,

    /// Filter runs by status
    #[arg(short, long, value_parser = clap::builder::PossibleValuesParser::new(RUN_STATUSES))]
    status: Option<String>,

    /// Filter runs by the event that triggered them (push, pull_request, ...)
    #[arg(short, long)]
    event: Option<String>,

    /// Filter runs by the user who triggered them
    #[arg(short, long)]
    user: Option<String>,
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    filter: RunFilterArgs,

    /// Filter runs by workflow file name (e.g. ci.yml)
    #[arg(short, long)]
    workflow: Option<String>,

    /// Maximum number of runs to fetch
    #[arg(short = 'L', long, default_value = "20")]
    limit: i64,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct ViewArgs {
    id: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct RerunArgs {
    id: i64,
}

#[derive(Args)]
struct WatchArgs {
    /// Run ID. If omitted (and no --commit), prompt to pick from in-progress runs.
    #[arg(conflicts_with = "commit")]
    id: Option<i64>,

    /// Watch the run(s) triggered by this commit (full SHA) instead of a run ID.
    #[arg(short = 'c', long, value_name = "SHA")]
    commit: Option<String>,

    /// With --commit: keep polling up to this many seconds for the commit's
    /// runs to appear (they may not exist yet right after a push).
    #[arg(long, value_name = "SECONDS", requires = "commit", default_value = "0")]
    wait_for_run: u64,

    /// Refresh interval in seconds.
    #[arg(short = 'i', long, default_value = "3")]
    interval: u64,

    /// Hide successful steps; show only relevant/failed steps.
    #[arg(long)]
    compact: bool,

    /// Exit non-zero if the run's conclusion is a failure. By default,
    /// `gtx run watch` exits 0 once the run reaches a terminal state,
    /// regardless of conclusion (matches `gh run watch`).
    #[arg(long = "exit-status")]
    exit_status: bool,
}

#[derive(Args)]
struct DownloadArgs {
    /// Run ID
    id: i64,

    /// Directory to extract artifacts into; each lands in `<dir>/<artifact name>/`
    #[arg(short, long, default_value = ".")]
    dir: String,
}

impl RunCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.action {
            RunAction::List(args) => list_runs(&self.repo, args).await,
            RunAction::View(args) => view_run(&self.repo, args).await,
            RunAction::Rerun(args) => rerun_run(&self.repo, args).await,
            RunAction::Watch(args) => watch_run(&self.repo, args).await,
            RunAction::Download(args) => download_artifacts(&self.repo, args).await,
        }
    }
}

const RUN_FIELDS: &[&str] = &[
    "id", "display_title", "status", "conclusion", "event", "head_branch",
    "head_sha", "path", "html_url", "started_at", "completed_at", "created_at",
    "updated_at",
];

/// Workflow runs carry `path` = `<workflow file>@<ref>`. Match a `--workflow`
/// argument given either as the bare file name or as a path to it.
fn run_matches_workflow(run: &gitea_api::types::ActionWorkflowRun, workflow: &str) -> bool {
    let wanted = workflow.rsplit('/').next().unwrap_or(workflow);
    run.path
        .as_deref()
        .and_then(|p| p.split('@').next())
        .is_some_and(|file| file == wanted)
}

/// Fetch runs matching `filter` (server side) and `keep` (client side),
/// paging until `limit` matches are collected or the server runs out.
async fn fetch_runs(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    filter: &RunFilterArgs,
    limit: i64,
    keep: impl Fn(&gitea_api::types::ActionWorkflowRun) -> bool,
) -> Result<Vec<gitea_api::types::ActionWorkflowRun>> {
    const PER_PAGE: i64 = 50;
    let mut runs = Vec::new();
    let mut page = 1;
    while (runs.len() as i64) < limit {
        let per_page = PER_PAGE.min(limit);
        let mut req = api
            .get_workflow_runs()
            .owner(owner)
            .repo(repo)
            .page(page)
            .limit(per_page);
        if let Some(v) = &filter.commit {
            req = req.head_sha(v.as_str());
        }
        if let Some(v) = &filter.branch {
            req = req.branch(v.as_str());
        }
        if let Some(v) = &filter.status {
            req = req.status(v.as_str());
        }
        if let Some(v) = &filter.event {
            req = req.event(v.as_str());
        }
        if let Some(v) = &filter.user {
            req = req.actor(v.as_str());
        }
        let batch = req
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner()
            .workflow_runs;
        let got = batch.len() as i64;
        runs.extend(batch.into_iter().filter(|r| keep(r)));
        if got < per_page {
            break;
        }
        page += 1;
    }
    runs.truncate(limit.max(0) as usize);
    Ok(runs)
}

async fn list_runs(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let runs = fetch_runs(
        &api,
        &repo_info.owner,
        &repo_info.name,
        &args.filter,
        args.limit,
        |run| args.workflow.as_deref().is_none_or(|w| run_matches_workflow(run, w)),
    )
    .await?;

    if args.json.is_json() {
        return crate::json::write_json(&args.json, &runs, RUN_FIELDS);
    }

    if runs.is_empty() {
        eprintln!("No workflow runs found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!("{:<8} {:<30} {:<12} {:<10} STARTED", "ID", "TITLE", "STATUS", "BRANCH");
    }

    for run in &runs {
        let id = run.id.unwrap_or(0);
        let title = run.display_title.as_deref().unwrap_or("");
        let truncated = if title.len() > 28 {
            format!("{}...", &title[..25])
        } else {
            title.to_string()
        };
        let status = run.status.as_deref().unwrap_or("");
        let branch = run.head_branch.as_deref().unwrap_or("");
        let started = run
            .started_at
            .map(relative_time)
            .unwrap_or_default();

        println!("{:<8} {:<30} {:<12} {:<10} {}", id, truncated, status, branch, started);
    }

    Ok(())
}

async fn view_run(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let run = api
        .get_workflow_run()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .run(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    let title = run.display_title.as_deref().unwrap_or("(unnamed)");
    let id = run.id.unwrap_or(0);
    let status = run.status.as_deref().unwrap_or("unknown");
    let conclusion = run.conclusion.as_deref().unwrap_or("");
    let branch = run.head_branch.as_deref().unwrap_or("");
    let event = run.event.as_deref().unwrap_or("");

    println!("{title} (#{id})");
    println!("Status: {status}{}", if conclusion.is_empty() { String::new() } else { format!(" ({conclusion})") });
    println!("Branch: {branch}");
    println!("Event: {event}");

    if let Some(ref url) = run.html_url {
        println!();
        println!("{url}");
    }

    Ok(())
}

async fn rerun_run(repo_args: &repo::RepoArgs, args: &RerunArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    api.rerun_workflow_run()
        .owner(&repo_info.owner)
        .repo(&repo_info.name)
        .run(args.id)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?;

    eprintln!("Rerun triggered for run #{}", args.id);
    Ok(())
}

fn format_run_picker_label(run: &gitea_api::types::ActionWorkflowRun) -> String {
    let id = run.id.unwrap_or(0);
    let status = run.status.as_deref().unwrap_or("");
    let conclusion = run.conclusion.as_deref().unwrap_or("");
    let icon = job_icon(status, conclusion);
    let branch = run.head_branch.as_deref().unwrap_or("");
    let title = run.display_title.as_deref().unwrap_or("(unnamed)");
    format!("{icon} #{id} {branch} {title}")
}

async fn pick_run(api: &gitea_api::Gitea, owner: &str, repo: &str) -> Result<i64> {
    let resp = api
        .get_workflow_runs()
        .owner(owner)
        .repo(repo)
        .page(1)
        .limit(30)
        .send()
        .await
        .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
        .into_inner();

    let (in_progress, recent) = partition_runs_for_picker(resp.workflow_runs);

    if in_progress.is_empty() && recent.is_empty() {
        eprintln!("No workflow runs found");
        std::process::exit(0);
    }

    // Build labels and a parallel id vector. inquire::Select returns the
    // chosen label; we look up its id by index.
    let mut labels: Vec<String> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    for run in in_progress.iter() {
        labels.push(format_run_picker_label(run));
        ids.push(run.id.unwrap_or(0));
    }
    if !in_progress.is_empty() && !recent.is_empty() {
        labels.push("───── recent ─────".to_string());
        ids.push(-1); // sentinel for the separator
    }
    for run in recent.iter().take(10) {
        labels.push(format_run_picker_label(run));
        ids.push(run.id.unwrap_or(0));
    }

    let chosen = match inquire::Select::new("Pick a run to watch:", labels.clone()).prompt() {
        Ok(s) => s,
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => {
            std::process::exit(130);
        }
        Err(e) => return Err(eyre::eyre!("{e}")),
    };

    let idx = labels
        .iter()
        .position(|l| l == &chosen)
        .ok_or_else(|| eyre::eyre!("internal: picked label not found"))?;
    let id = ids[idx];
    if id < 0 {
        // User somehow selected the separator; treat as cancel.
        std::process::exit(130);
    }
    Ok(id)
}

async fn watch_run(repo_args: &repo::RepoArgs, args: &WatchArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo_name) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let run_ids = match (args.id, &args.commit) {
        (Some(id), _) => vec![id],
        (None, Some(sha)) => wait_for_commit_runs(&api, owner, repo_name, sha, args).await?,
        (None, None) => vec![pick_run(&api, owner, repo_name).await?],
    };

    let mut any_failed = false;
    for run_id in run_ids {
        any_failed |= watch_one(&api, owner, repo_name, run_id, args).await?;
    }
    if args.exit_status && any_failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Poll for the runs `sha` triggered, for up to `--wait-for-run` seconds.
async fn wait_for_commit_runs(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    sha: &str,
    args: &WatchArgs,
) -> Result<Vec<i64>> {
    let filter = RunFilterArgs {
        commit: Some(sha.to_string()),
        ..Default::default()
    };
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(args.wait_for_run);
    loop {
        let runs = fetch_runs(api, owner, repo, &filter, 100, |_| true).await?;
        if !runs.is_empty() {
            // The API lists newest first; watch in trigger order.
            return Ok(runs.iter().rev().filter_map(|r| r.id).collect());
        }
        if std::time::Instant::now() >= deadline {
            eyre::bail!("No workflow runs found for commit {sha}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(args.interval)).await;
    }
}

/// Watch one run until it reaches a terminal state. Returns whether it failed.
async fn watch_one(
    api: &gitea_api::Gitea,
    owner: &str,
    repo_name: &str,
    run_id: i64,
    args: &WatchArgs,
) -> Result<bool> {
    let is_tty = crate::issues::atty_check();
    let mut prev_lines: Option<usize> = None;

    loop {
        let run = api
            .get_workflow_run()
            .owner(owner)
            .repo(repo_name)
            .run(run_id)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner();

        let jobs = api
            .list_workflow_run_jobs()
            .owner(owner)
            .repo(repo_name)
            .run(run_id)
            .send()
            .await
            .map_err(|e| eyre::eyre!("{}", gitea_api::GiteaError::from(e)))?
            .into_inner()
            .jobs;

        let body = render_run_state(&run, &jobs, args.compact);
        redraw(&mut prev_lines, &body, is_tty);

        let status = run.status.as_deref().unwrap_or("");
        if is_terminal_status(status) {
            let conclusion = run.conclusion.as_deref().unwrap_or("");
            return Ok(is_failure_conclusion(status) || is_failure_conclusion(conclusion));
        }

        tokio::time::sleep(std::time::Duration::from_secs(args.interval)).await;
    }
}

/// `<out_dir>/<name>`, refusing artifact names that would land anywhere else
/// (`..`, absolute paths, nested separators).
fn artifact_dir(out_dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf> {
    let mut parts = std::path::Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(out_dir.join(name)),
        _ => Err(eyre::eyre!("Refusing to download artifact with unsafe name {name:?}")),
    }
}

async fn download_artifacts(repo_args: &repo::RepoArgs, args: &DownloadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // List artifacts for this run
    let resp = api
        .raw_get(&format!("repos/{owner}/{repo}/actions/runs/{}/artifacts", args.id))
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    let data: serde_json::Value = serde_json::from_str(&resp)?;

    let artifacts = data
        .get("artifacts")
        .and_then(|a| a.as_array())
        .ok_or_else(|| eyre::eyre!("No artifacts found for run #{}", args.id))?;

    if artifacts.is_empty() {
        eprintln!("No artifacts found for run #{}", args.id);
        return Ok(());
    }

    let out_dir = std::path::Path::new(&args.dir);
    std::fs::create_dir_all(out_dir)?;

    for artifact in artifacts {
        let name = artifact["name"].as_str().unwrap_or("artifact");
        let artifact_id = artifact["id"]
            .as_i64()
            .ok_or_else(|| eyre::eyre!("Artifact missing ID"))?;

        if artifact["expired"].as_bool() == Some(true) {
            eprintln!("Skipping expired artifact {name}");
            continue;
        }
        let dest = artifact_dir(out_dir, name)?;

        // `artifacts/{id}` is the metadata; the bytes are at `/zip`, which
        // Gitea answers with a redirect to a signed blob URL. `download`
        // only attaches the token for this instance's origin, and reqwest
        // drops it when a redirect leaves that origin.
        let url = api.url_for(&format!(
            "repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip"
        ));
        let resp = api.download(&url).await.map_err(|e| eyre::eyre!("{e}"))?;
        let resp = gitea_api::error_for_status(resp)
            .await
            .map_err(|e| eyre::eyre!("Failed to download {name}: {e}"))?;
        let bytes = resp.bytes().await?;

        // Like `gh run download`, unpack each artifact into `<dir>/<name>/`.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .map_err(|e| eyre::eyre!("Downloaded {name} is not a zip archive: {e}"))?;
        archive
            .extract(&dest)
            .map_err(|e| eyre::eyre!("Failed to extract {name}: {e}"))?;
        eprintln!(
            "Downloaded {name} ({} bytes) to {}",
            bytes.len(),
            dest.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_dir_rejects_names_that_escape_out_dir() {
        let out = std::path::Path::new("/tmp/out");
        assert_eq!(artifact_dir(out, "logs").unwrap(), out.join("logs"));
        for bad in ["..", "../x", "a/b", "/etc", ".", ""] {
            assert!(artifact_dir(out, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn terminal_status_recognises_completed_and_outcome_aliases() {
        assert!(is_terminal_status("completed"));
        assert!(is_terminal_status("success"));
        assert!(is_terminal_status("failure"));
        assert!(is_terminal_status("cancelled"));
        assert!(is_terminal_status("skipped"));
        assert!(is_terminal_status("timed_out"));
        assert!(is_terminal_status("action_required"));
    }

    #[test]
    fn terminal_status_rejects_in_flight_states() {
        assert!(!is_terminal_status("in_progress"));
        assert!(!is_terminal_status("queued"));
        assert!(!is_terminal_status("waiting"));
        assert!(!is_terminal_status(""));
    }

    #[test]
    fn failure_conclusion_truth_table() {
        assert!(is_failure_conclusion("failure"));
        assert!(is_failure_conclusion("cancelled"));
        assert!(is_failure_conclusion("timed_out"));
        assert!(is_failure_conclusion("action_required"));

        assert!(!is_failure_conclusion("success"));
        assert!(!is_failure_conclusion("skipped"));
        assert!(!is_failure_conclusion(""));
        assert!(!is_failure_conclusion("in_progress"));
    }

    fn make_run(id: i64, title: &str, status: &str) -> gitea_api::types::ActionWorkflowRun {
        gitea_api::types::ActionWorkflowRun {
            id: Some(id),
            display_title: Some(title.to_string()),
            status: Some(status.to_string()),
            ..Default::default()
        }
    }

    fn make_step(name: &str, status: &str, conclusion: Option<&str>) -> gitea_api::types::ActionWorkflowStep {
        gitea_api::types::ActionWorkflowStep {
            name: Some(name.to_string()),
            status: Some(status.to_string()),
            conclusion: conclusion.map(str::to_string),
            ..Default::default()
        }
    }

    fn make_job(
        name: &str,
        status: &str,
        conclusion: Option<&str>,
        steps: Vec<gitea_api::types::ActionWorkflowStep>,
    ) -> gitea_api::types::ActionWorkflowJob {
        gitea_api::types::ActionWorkflowJob {
            name: Some(name.to_string()),
            status: Some(status.to_string()),
            conclusion: conclusion.map(str::to_string),
            steps,
            ..Default::default()
        }
    }

    #[test]
    fn render_default_shows_every_step() {
        let run = make_run(42, "feat: hello", "in_progress");
        let jobs = vec![
            make_job(
                "build",
                "completed",
                Some("success"),
                vec![
                    make_step("checkout", "completed", Some("success")),
                    make_step("cargo test", "completed", Some("success")),
                ],
            ),
            make_job(
                "lint",
                "in_progress",
                None,
                vec![
                    make_step("checkout", "completed", Some("success")),
                    make_step("clippy", "in_progress", None),
                ],
            ),
        ];

        let out = render_run_state(&run, &jobs, false);

        assert!(out.contains("Run #42 — feat: hello"), "header missing: {out}");
        assert!(out.contains("Status: in_progress"), "status missing: {out}");
        assert!(out.contains("✓ build"));
        assert!(out.contains("✓ checkout"));
        assert!(out.contains("✓ cargo test"));
        assert!(out.contains("● lint"));
        assert!(out.contains("● clippy"));
    }

    #[test]
    fn render_default_shows_queued_job() {
        let run = make_run(42, "queue check", "in_progress");
        let jobs = vec![make_job(
            "release",
            "queued",
            None,
            vec![],
        )];

        let out = render_run_state(&run, &jobs, false);

        assert!(out.contains("○ release"), "queued job missing icon: {out}");
        assert!(out.contains("(queued)"), "queued status label missing: {out}");
    }

    #[test]
    fn render_compact_hides_fully_successful_jobs() {
        let run = make_run(42, "feat: hello", "in_progress");
        let jobs = vec![
            make_job(
                "build",
                "completed",
                Some("success"),
                vec![make_step("cargo test", "completed", Some("success"))],
            ),
            make_job(
                "lint",
                "completed",
                Some("failure"),
                vec![
                    make_step("checkout", "completed", Some("success")),
                    make_step("clippy", "completed", Some("failure")),
                ],
            ),
        ];

        let out = render_run_state(&run, &jobs, true);

        // The all-green build job is hidden entirely.
        assert!(!out.contains("build"), "compact should hide successful job: {out}");
        // The failing lint job is shown.
        assert!(out.contains("✗ lint"), "lint job should appear: {out}");
        // Within the failing job, only the failed step shows.
        assert!(out.contains("✗ clippy"), "failed step should appear: {out}");
        assert!(!out.contains("✓ checkout"), "passing step should be hidden: {out}");
    }

    #[test]
    fn render_compact_collapses_all_green() {
        let run = make_run(42, "all green", "in_progress");
        let jobs = vec![make_job(
            "build",
            "completed",
            Some("success"),
            vec![make_step("cargo test", "completed", Some("success"))],
        )];

        let out = render_run_state(&run, &jobs, true);

        assert!(out.contains("Run #42"));
        assert!(out.contains("(all steps passing so far)"), "fallback line missing: {out}");
        assert!(!out.contains("build"), "no jobs should appear in compact all-green: {out}");
    }

    #[test]
    fn render_compact_shows_in_progress_job() {
        let run = make_run(42, "in flight", "in_progress");
        let jobs = vec![make_job(
            "build",
            "in_progress",
            None,
            vec![
                make_step("checkout", "completed", Some("success")),
                make_step("cargo test", "in_progress", None),
            ],
        )];

        let out = render_run_state(&run, &jobs, true);

        assert!(out.contains("● build"), "in_progress job missing: {out}");
        assert!(out.contains("● cargo test"), "in_progress step missing: {out}");
        assert!(!out.contains("✓ checkout"), "successful step should be hidden in compact: {out}");
    }

    #[test]
    fn partition_runs_separates_in_progress_from_recent() {
        let runs = vec![
            make_run(1, "old success", "completed"),
            make_run(2, "queued", "queued"),
            make_run(3, "running", "in_progress"),
            make_run(4, "old failure", "failure"),
            make_run(5, "waiting", "waiting"),
        ];

        let (in_progress, recent) = partition_runs_for_picker(runs);

        let in_progress_ids: Vec<_> = in_progress.iter().map(|r| r.id.unwrap()).collect();
        let recent_ids: Vec<_> = recent.iter().map(|r| r.id.unwrap()).collect();

        assert_eq!(in_progress_ids, vec![2, 3, 5], "in-progress includes waiting/queued/in_progress");
        assert_eq!(recent_ids, vec![1, 4], "recent is everything else");
    }

    #[test]
    fn workflow_match_uses_file_name_before_ref() {
        let run = gitea_api::types::ActionWorkflowRun {
            path: Some("ci.yml@refs/heads/main".into()),
            ..Default::default()
        };
        assert!(run_matches_workflow(&run, "ci.yml"));
        assert!(run_matches_workflow(&run, ".forgejo/workflows/ci.yml"));
        assert!(!run_matches_workflow(&run, "ci"));
        assert!(!run_matches_workflow(&run, "release.yml"));
        assert!(!run_matches_workflow(&Default::default(), "ci.yml"));
    }

    #[test]
    fn partition_handles_missing_status_field() {
        // Status: None → treated as recent (not in-progress).
        let runs = vec![gitea_api::types::ActionWorkflowRun {
            id: Some(99),
            ..Default::default()
        }];

        let (in_progress, recent) = partition_runs_for_picker(runs);

        assert!(in_progress.is_empty());
        assert_eq!(recent.len(), 1);
    }
}
