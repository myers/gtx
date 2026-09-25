use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Subcommand};
use eyre::Result;

use crate::config::Config;
use crate::issues::{atty_check, relative_time};
use crate::json::{Field, field, gh};
use crate::repo;
use gitea_api::types::ActionWorkflowRun;

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed"
            | "success"
            | "failure"
            | "cancelled"
            | "skipped"
            | "timed_out"
            | "action_required",
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
    let visible = matches!(
        conclusion,
        "failure" | "cancelled" | "timed_out" | "action_required"
    ) || matches!(status, "in_progress" | "queued" | "waiting");
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
    /// Cancel a workflow run
    Cancel(CancelArgs),
    /// Watch a workflow run (poll until complete, show logs)
    Watch(WatchArgs),
    /// Download artifacts from a workflow run
    Download(DownloadArgs),
}

/// Run statuses Gitea's `status` query parameter accepts.
const RUN_STATUSES: &[&str] = &[
    "queued",
    "in_progress",
    "completed",
    "pending",
    "waiting",
    "requested",
    "action_required",
    "success",
    "failure",
    "skipped",
    "neutral",
    "cancelled",
    "timed_out",
];

/// Server-side filters for the workflow-run list endpoint.
#[derive(Args, Clone, Default)]
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
    /// Run ID. If omitted (and no --job), prompt to pick a recent run.
    id: Option<i64>,

    /// View a specific job ID from a run
    #[arg(short, long, value_name = "JOB_ID")]
    job: Option<i64>,

    /// View full log for either a run or specific job
    #[arg(long, conflicts_with = "log_failed")]
    log: bool,

    /// View the log for any failed steps in a run or specific job
    #[arg(long)]
    log_failed: bool,

    /// Exit with non-zero status if run failed
    #[arg(long)]
    exit_status: bool,

    #[command(flatten)]
    json: crate::json::JsonArgs,
}

#[derive(Args)]
struct CancelArgs {
    /// Run ID. If omitted, prompt to pick an in-progress run.
    id: Option<i64>,
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
            RunAction::Cancel(args) => cancel_run(&self.repo, args).await,
            RunAction::Watch(args) => watch_run(&self.repo, args).await,
            RunAction::Download(args) => download_artifacts(&self.repo, args).await,
        }
    }
}

/// The workflow file name of a run (its `path` is `<file>@<ref>`).
fn run_workflow_name(run: &ActionWorkflowRun) -> Option<&str> {
    let file = run.path.as_deref()?.split('@').next()?;
    Some(file.rsplit('/').next().unwrap_or(file))
}

/// A run's branch as gh reports it. Gitea leaves `head_branch` unset on
/// pull_request runs (their ref is `refs/pull/N/head`); gh's is the PR's
/// head branch, which Gitea sends as `pull_requests[0].head.ref`.
fn run_branch(run: &ActionWorkflowRun) -> Option<&str> {
    run.head_branch
        .as_deref()
        .filter(|b| !b.is_empty())
        .or_else(|| run.pull_requests.first()?.head.as_ref()?.ref_.as_deref())
}

/// A run/job/step time, or `None` when Gitea reports it unset: the Unix
/// epoch (e.g. `started_at` of a queued run) or Go's zero time.
fn set_time(t: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    t.filter(|t| t.timestamp() > 0)
}

/// A run/job/step time as gh prints it: unset times are Go's zero time,
/// `0001-01-01T00:00:00Z`, not `null`.
fn run_time(t: Option<DateTime<Utc>>) -> serde_json::Value {
    let zero = NaiveDate::from_ymd_opt(1, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc());
    gh::time(set_time(t).or(zero))
}

/// gh's `run list/view --json` fields, plus any `$extra` fields. A macro so
/// the same getters serve `ActionWorkflowRun` and [`RunView`]. Gitea has no
/// numeric workflow ID: `name`/`workflowName` are the workflow's file name.
macro_rules! run_fields {
    ($($extra:expr),* $(,)?) => { &[
    field("attempt", |r| gh::v(r.run_attempt)),
    field("conclusion", |r| gh::v(r.conclusion.as_deref().unwrap_or(""))),
    field("createdAt", |r| run_time(r.created_at)),
    field("databaseId", |r| gh::v(r.id)),
    field("displayTitle", |r| gh::v(&r.display_title)),
    field("event", |r| gh::v(&r.event)),
    field("headBranch", |r| gh::v(run_branch(r))),
    field("headSha", |r| gh::v(&r.head_sha)),
    field("name", |r| gh::v(run_workflow_name(r))),
    field("number", |r| gh::v(r.run_number)),
    field("startedAt", |r| run_time(r.started_at)),
    field("status", |r| gh::v(&r.status)),
    field("updatedAt", |r| run_time(r.updated_at)),
    field("url", |r| gh::v(&r.html_url)),
    field("workflowName", |r| gh::v(run_workflow_name(r))),
    $($extra),*
    ] };
}

const RUN_FIELDS: &[Field<ActionWorkflowRun>] = run_fields!();

/// A run plus its jobs, fetched only when `run view --json jobs` asks.
struct RunView {
    run: ActionWorkflowRun,
    jobs: Vec<gitea_api::types::ActionWorkflowJob>,
}

impl std::ops::Deref for RunView {
    type Target = ActionWorkflowRun;
    fn deref(&self) -> &ActionWorkflowRun {
        &self.run
    }
}

const RUN_VIEW_FIELDS: &[Field<RunView>] = run_fields![field("jobs", |v| {
    serde_json::Value::Array(v.jobs.iter().map(gh_job).collect())
})];

/// gh's job object for `run view --json jobs`.
fn gh_job(j: &gitea_api::types::ActionWorkflowJob) -> serde_json::Value {
    let steps: Vec<_> = j
        .steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "completedAt": run_time(s.completed_at),
                "conclusion": s.conclusion.as_deref().unwrap_or(""),
                "name": s.name,
                "number": s.number,
                "startedAt": run_time(s.started_at),
                "status": s.status,
            })
        })
        .collect();
    serde_json::json!({
        "completedAt": run_time(j.completed_at),
        "conclusion": j.conclusion.as_deref().unwrap_or(""),
        "databaseId": j.id,
        "name": j.name,
        "startedAt": run_time(j.started_at),
        "status": j.status,
        "steps": steps,
        "url": j.html_url,
    })
}

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

/// [`fetch_runs`] for `--branch`. Gitea's `branch` query matches only
/// `refs/heads/<branch>`, never a pull_request run's `refs/pull/N/head`, so
/// unless `--event` rules them out, also page through pull_request runs
/// (unfiltered by branch) keeping those whose PR head branch is `branch`.
/// Both lists come newest first; merge them by ID and keep the newest
/// `limit`, which lie within the first `limit` of each.
async fn fetch_branch_runs(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    filter: &RunFilterArgs,
    branch: &str,
    limit: i64,
    keep: impl Fn(&ActionWorkflowRun) -> bool,
) -> Result<Vec<ActionWorkflowRun>> {
    let mut runs = fetch_runs(api, owner, repo, filter, limit, &keep).await?;
    let pr_event = match filter.event.as_deref() {
        None => Some("pull_request"),
        Some(e) if e.starts_with("pull_request") => Some(e),
        Some(_) => None,
    };
    if let Some(event) = pr_event {
        let pr_filter = RunFilterArgs {
            branch: None,
            event: Some(event.to_string()),
            ..filter.clone()
        };
        let pr_runs = fetch_runs(api, owner, repo, &pr_filter, limit, |r| {
            r.head_branch.as_deref().is_none_or(str::is_empty)
                && run_branch(r) == Some(branch)
                && keep(r)
        })
        .await?;
        runs.extend(pr_runs);
        runs.sort_by_key(|r| std::cmp::Reverse(r.id));
        runs.dedup_by_key(|r| r.id);
        runs.truncate(limit.max(0) as usize);
    }
    Ok(runs)
}

async fn list_runs(repo_args: &repo::RepoArgs, args: &ListArgs) -> Result<()> {
    let json = args.json.select(RUN_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;

    let keep = |run: &ActionWorkflowRun| {
        args.workflow
            .as_deref()
            .is_none_or(|w| run_matches_workflow(run, w))
    };
    let (owner, name) = (repo_info.owner.as_str(), repo_info.name.as_str());
    let runs = match &args.filter.branch {
        Some(branch) => {
            fetch_branch_runs(&api, owner, name, &args.filter, branch, args.limit, keep).await?
        }
        None => fetch_runs(&api, owner, name, &args.filter, args.limit, keep).await?,
    };

    if let Some(json) = json {
        return json.write_list(&runs);
    }

    if runs.is_empty() {
        eprintln!("No workflow runs found");
        return Ok(());
    }

    let is_tty = atty_check();
    if is_tty {
        println!(
            "{:<8} {:<30} {:<12} {:<10} AGE",
            "ID", "TITLE", "STATUS", "BRANCH"
        );
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
        let branch = run_branch(run).unwrap_or("");
        let age = set_time(run.created_at)
            .map(relative_time)
            .unwrap_or_default();

        println!(
            "{:<8} {:<30} {:<12} {:<10} {}",
            id, truncated, status, branch, age
        );
    }

    Ok(())
}

fn api_err(e: impl Into<gitea_api::GiteaError>) -> eyre::Report {
    eyre::eyre!("{}", e.into())
}

async fn view_run(repo_args: &repo::RepoArgs, args: &ViewArgs) -> Result<()> {
    let json = args.json.select(RUN_VIEW_FIELDS)?;
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, name) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // Like gh, `--job` picks the run: the job's own.
    let selected_job = match args.job {
        Some(job_id) => Some(
            api.get_workflow_job()
                .owner(owner)
                .repo(name)
                .job_id(job_id.to_string())
                .send()
                .await
                .map_err(api_err)?
                .into_inner(),
        ),
        None => None,
    };
    let run_id = match (selected_job.as_ref().and_then(|j| j.run_id), args.id) {
        (Some(id), _) | (None, Some(id)) => id,
        (None, None) if atty_check() => {
            pick_run(&api, owner, name, "Select a workflow run", false).await?
        }
        (None, None) => eyre::bail!("run or job ID required when not running interactively"),
    };

    let run = api
        .get_workflow_run()
        .owner(owner)
        .repo(name)
        .run(run_id)
        .send()
        .await
        .map_err(api_err)?
        .into_inner();

    let fetch_jobs = || async {
        Ok::<_, eyre::Report>(
            api.list_workflow_run_jobs()
                .owner(owner)
                .repo(name)
                .run(run_id)
                .send()
                .await
                .map_err(api_err)?
                .into_inner()
                .jobs,
        )
    };

    if let Some(json) = json {
        let jobs = if json.wants("jobs") {
            fetch_jobs().await?
        } else {
            Vec::new()
        };
        return json.write_one(&RunView { run, jobs });
    }

    if args.log || args.log_failed {
        let jobs = match selected_job {
            Some(job) => {
                if !is_terminal_status(job.status.as_deref().unwrap_or("")) {
                    eyre::bail!(
                        "job {} is still in progress; logs will be available when it is complete",
                        job.id.unwrap_or(0)
                    );
                }
                vec![job]
            }
            None => {
                if !is_terminal_status(run.status.as_deref().unwrap_or("")) {
                    eyre::bail!(
                        "run {run_id} is still in progress; logs will be available when it is complete"
                    );
                }
                fetch_jobs().await?
            }
        };
        return print_run_log(&api, owner, name, &jobs, args.log_failed).await;
    }

    let jobs = match &selected_job {
        Some(job) => vec![job.clone()],
        None => fetch_jobs().await?,
    };
    let artifacts = if selected_job.is_none() {
        let resp = api
            .raw_get(&format!(
                "repos/{owner}/{name}/actions/runs/{run_id}/artifacts"
            ))
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;
        let data: serde_json::Value = serde_json::from_str(&resp)?;
        data["artifacts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let name = a["name"].as_str().unwrap_or("").to_string();
                (name, a["expired"].as_bool() == Some(true))
            })
            .collect()
    } else {
        Vec::new()
    };
    // gh's `-v` is gtx's global `-v`/`--verbose` (which also turns on HTTP
    // transcripts on stderr).
    let verbose = gitea_api::verbose::config().is_some_and(|c| c.is_on());
    let view = TextView {
        run: &run,
        jobs: &jobs,
        selected_job: selected_job.as_ref(),
        artifacts: &artifacts,
        verbose,
    };
    print!("{}", view.render());

    let failed = match &selected_job {
        Some(job) => is_failure_conclusion(outcome(&job.status, &job.conclusion).1),
        None => is_failure_conclusion(outcome(&run.status, &run.conclusion).1),
    };
    if args.exit_status && failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Whether a run/job/step is completed, and its conclusion. Gitea may report
/// an outcome (`success`, `failure`, ...) as the status itself.
fn outcome<'a>(status: &'a Option<String>, conclusion: &'a Option<String>) -> (bool, &'a str) {
    let status = status.as_deref().unwrap_or("");
    let conclusion = conclusion.as_deref().unwrap_or("");
    let completed = is_terminal_status(status);
    match (completed, conclusion, status) {
        (true, "", "completed") => (true, ""),
        (true, "", s) => (true, s),
        (c, conc, _) => (c, conc),
    }
}

/// gh's status symbol: ✓ success, - skipped/neutral, X other completed
/// outcomes, * not yet completed.
fn gh_symbol(status: &Option<String>, conclusion: &Option<String>) -> &'static str {
    match outcome(status, conclusion) {
        (false, _) => "*",
        (true, "success") => "✓",
        (true, "skipped" | "neutral") => "-",
        (true, _) => "X",
    }
}

/// A duration in whole seconds as Go's `time.Duration` prints it (`1m3s`).
fn go_duration(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m{s}s"),
        _ => format!("{h}h{m}m{s}s"),
    }
}

/// gh's text `run view`.
struct TextView<'a> {
    run: &'a ActionWorkflowRun,
    jobs: &'a [gitea_api::types::ActionWorkflowJob],
    selected_job: Option<&'a gitea_api::types::ActionWorkflowJob>,
    /// `(name, expired)`
    artifacts: &'a [(String, bool)],
    verbose: bool,
}

impl TextView<'_> {
    fn render(&self) -> String {
        use std::fmt::Write;
        let run = self.run;
        let id = run.id.unwrap_or(0);
        let url = run.html_url.as_deref().unwrap_or("");
        let mut out = String::new();

        let pr = match run.pull_requests.first().and_then(|p| p.number) {
            Some(n) => format!(" #{n}"),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "\n{} {} {}{pr} · {id}",
            gh_symbol(&run.status, &run.conclusion),
            run_branch(run).unwrap_or(""),
            run_workflow_name(run).unwrap_or(""),
        );
        let ago = set_time(run.started_at)
            .or(set_time(run.created_at))
            .map(relative_time)
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "Triggered via {} {ago}\n",
            run.event.as_deref().unwrap_or("")
        );

        let (_, conclusion) = outcome(&run.status, &run.conclusion);
        if self.jobs.is_empty() && matches!(conclusion, "failure" | "startup_failure") {
            let _ = writeln!(
                out,
                "X This run likely failed because of a workflow file issue.\n"
            );
            let _ = writeln!(out, "For more information, see: {url}");
            return out;
        }

        match self.selected_job {
            None => {
                out.push_str("JOBS\n");
                out.push_str(&render_jobs(self.jobs, self.verbose));
            }
            Some(job) => out.push_str(&render_jobs(std::slice::from_ref(job), true)),
        }

        match self.selected_job {
            None => {
                if !self.artifacts.is_empty() {
                    out.push_str("\nARTIFACTS\n");
                    for (name, expired) in self.artifacts {
                        let badge = if *expired { " (expired)" } else { "" };
                        let _ = writeln!(out, "{name}{badge}");
                    }
                }
                out.push('\n');
                if is_failure_conclusion(conclusion) {
                    let _ = writeln!(
                        out,
                        "To see what failed, try: gtx run view {id} --log-failed"
                    );
                } else if let [job] = self.jobs {
                    let _ = writeln!(
                        out,
                        "For more information about the job, try: gtx run view --job={}",
                        job.id.unwrap_or(0)
                    );
                } else {
                    out.push_str(
                        "For more information about a job, try: gtx run view --job=<job-id>\n",
                    );
                }
            }
            Some(job) => {
                out.push('\n');
                let job_id = job.id.unwrap_or(0);
                if is_failure_conclusion(outcome(&job.status, &job.conclusion).1) {
                    let _ = writeln!(
                        out,
                        "To see the logs for the failed steps, try: gtx run view --log-failed --job={job_id}"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "To see the full job log, try: gtx run view --log --job={job_id}"
                    );
                }
            }
        }
        let _ = writeln!(out, "View this run on Gitea: {url}");
        out
    }
}

/// gh's JOBS list: one line per job, plus its steps when `verbose` or the
/// job failed.
fn render_jobs(jobs: &[gitea_api::types::ActionWorkflowJob], verbose: bool) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for job in jobs {
        let elapsed = match (set_time(job.started_at), set_time(job.completed_at)) {
            (Some(start), Some(end)) if end >= start => {
                format!(" in {}", go_duration((end - start).num_seconds()))
            }
            _ => String::new(),
        };
        let _ = writeln!(
            out,
            "{} {}{elapsed} (ID {})",
            gh_symbol(&job.status, &job.conclusion),
            job.name.as_deref().unwrap_or(""),
            job.id.unwrap_or(0),
        );
        if verbose || is_failure_conclusion(outcome(&job.status, &job.conclusion).1) {
            for step in &job.steps {
                let _ = writeln!(
                    out,
                    "  {} {}",
                    gh_symbol(&step.status, &step.conclusion),
                    step.name.as_deref().unwrap_or("")
                );
            }
        }
    }
    out
}

/// The step name for log lines before any step started.
const SETUP_STEP: &str = "Set up job";

/// Split a Gitea job log into `(step, line)` pairs, `step` indexing
/// `job.steps` or `None` for [`SETUP_STEP`].
///
/// Gitea's log API returns the job's whole log with no step markers (the
/// server's per-step offsets aren't exposed). Each line starts with the
/// runner's RFC 3339 timestamp, and steps carry start times truncated to the
/// second, recorded a little after their first lines. So a line belongs to
/// the last step to start at or before its timestamp rounded up to the
/// second; untimestamped lines follow the previous line.
fn split_job_log<'a>(
    job: &gitea_api::types::ActionWorkflowJob,
    log: &'a str,
) -> Vec<(Option<usize>, &'a str)> {
    let starts: Vec<(usize, i64)> = job
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((i, set_time(s.started_at)?.timestamp())))
        .collect();
    let mut step = None;
    log.lines()
        .map(|line| {
            let ts = line
                .split_once(' ')
                .and_then(|(ts, _)| DateTime::parse_from_rfc3339(ts).ok());
            if let Some(ts) = ts {
                let secs = ts.timestamp() + i64::from(ts.timestamp_subsec_nanos() > 0);
                step = starts
                    .iter()
                    .filter(|(_, start)| *start <= secs)
                    .map(|(i, _)| *i)
                    .next_back();
            }
            (step, line)
        })
        .collect()
}

/// gh's `--log`/`--log-failed` output: each line as `JOB\tSTEP\tLINE`.
async fn print_run_log(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    jobs: &[gitea_api::types::ActionWorkflowJob],
    failed_only: bool,
) -> Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for job in jobs {
        let job_failed = is_failure_conclusion(outcome(&job.status, &job.conclusion).1);
        // A job that never started has no log.
        if (failed_only && !job_failed) || set_time(job.started_at).is_none() {
            continue;
        }
        let step_failed = |s: &gitea_api::types::ActionWorkflowStep| {
            is_failure_conclusion(outcome(&s.status, &s.conclusion).1)
        };
        // Set-up lines count as failed when the job failed before any
        // step that ran failed (e.g. its container never came up).
        let setup_failed = job_failed
            && !job
                .steps
                .iter()
                .any(|s| set_time(s.started_at).is_some() && step_failed(s));
        let log = api
            .raw_get(&format!(
                "repos/{owner}/{repo}/actions/jobs/{}/logs",
                job.id.unwrap_or(0)
            ))
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;
        let job_name = job.name.as_deref().unwrap_or("");
        for (step, line) in split_job_log(job, &log) {
            let (name, failed) = match step {
                Some(i) => {
                    let s = &job.steps[i];
                    (s.name.as_deref().unwrap_or(""), step_failed(s))
                }
                None => (SETUP_STEP, setup_failed),
            };
            if failed_only && !failed {
                continue;
            }
            writeln!(out, "{job_name}\t{name}\t{line}")?;
        }
    }
    Ok(())
}

async fn cancel_run(repo_args: &repo::RepoArgs, args: &CancelArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, name) = (repo_info.owner.as_str(), repo_info.name.as_str());

    let id = match args.id {
        Some(id) => id,
        None if atty_check() => pick_run(&api, owner, name, "Select a workflow run", true).await?,
        None => eyre::bail!("run ID required when not running interactively"),
    };
    let resp = api
        .raw_request(
            gitea_api::Method::POST,
            &format!("repos/{owner}/{name}/actions/runs/{id}/cancel"),
            None,
        )
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    if resp.status() == reqwest::StatusCode::CONFLICT {
        eyre::bail!("Cannot cancel a workflow run that is completed");
    }
    gitea_api::error_for_status(resp)
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    println!("✓ Request to cancel workflow {id} submitted.");
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
    let branch = run_branch(run).unwrap_or("");
    let title = run.display_title.as_deref().unwrap_or("(unnamed)");
    format!("{icon} #{id} {branch} {title}")
}

/// Prompt for a run: in-progress runs first, then (unless
/// `in_progress_only`) recent ones.
async fn pick_run(
    api: &gitea_api::Gitea,
    owner: &str,
    repo: &str,
    prompt: &str,
    in_progress_only: bool,
) -> Result<i64> {
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

    let (in_progress, mut recent) = partition_runs_for_picker(resp.workflow_runs);
    if in_progress_only {
        if in_progress.is_empty() {
            eyre::bail!("found no in progress runs to cancel");
        }
        recent.clear();
    }

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

    let chosen = match inquire::Select::new(prompt, labels.clone()).prompt() {
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
        (None, None) => {
            vec![pick_run(&api, owner, repo_name, "Pick a run to watch:", false).await?]
        }
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(args.wait_for_run);
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
        _ => Err(eyre::eyre!(
            "Refusing to download artifact with unsafe name {name:?}"
        )),
    }
}

async fn download_artifacts(repo_args: &repo::RepoArgs, args: &DownloadArgs) -> Result<()> {
    let config = Config::load()?;
    let api = config.client()?;
    let repo_info = repo::resolve_repo(repo_args.repo.as_deref(), &config.url)?;
    let (owner, repo) = (repo_info.owner.as_str(), repo_info.name.as_str());

    // List artifacts for this run
    let resp = api
        .raw_get(&format!(
            "repos/{owner}/{repo}/actions/runs/{}/artifacts",
            args.id
        ))
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

    fn make_step(
        name: &str,
        status: &str,
        conclusion: Option<&str>,
    ) -> gitea_api::types::ActionWorkflowStep {
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

        assert!(
            out.contains("Run #42 — feat: hello"),
            "header missing: {out}"
        );
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
        let jobs = vec![make_job("release", "queued", None, vec![])];

        let out = render_run_state(&run, &jobs, false);

        assert!(out.contains("○ release"), "queued job missing icon: {out}");
        assert!(
            out.contains("(queued)"),
            "queued status label missing: {out}"
        );
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
        assert!(
            !out.contains("build"),
            "compact should hide successful job: {out}"
        );
        // The failing lint job is shown.
        assert!(out.contains("✗ lint"), "lint job should appear: {out}");
        // Within the failing job, only the failed step shows.
        assert!(out.contains("✗ clippy"), "failed step should appear: {out}");
        assert!(
            !out.contains("✓ checkout"),
            "passing step should be hidden: {out}"
        );
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
        assert!(
            out.contains("(all steps passing so far)"),
            "fallback line missing: {out}"
        );
        assert!(
            !out.contains("build"),
            "no jobs should appear in compact all-green: {out}"
        );
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
        assert!(
            out.contains("● cargo test"),
            "in_progress step missing: {out}"
        );
        assert!(
            !out.contains("✓ checkout"),
            "successful step should be hidden in compact: {out}"
        );
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

        assert_eq!(
            in_progress_ids,
            vec![2, 3, 5],
            "in-progress includes waiting/queued/in_progress"
        );
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

    #[test]
    fn picker_label_shows_pull_request_head_branch() {
        let run: gitea_api::types::ActionWorkflowRun = serde_json::from_str(
            r#"{"id":281,"status":"completed","conclusion":"failure","display_title":"t",
                "event":"pull_request","pull_requests":[{"head":{"ref":"feature"}}]}"#,
        )
        .unwrap();
        assert_eq!(format_run_picker_label(&run), "✗ #281 feature t");
    }

    #[test]
    fn go_duration_matches_go() {
        assert_eq!(go_duration(0), "0s");
        assert_eq!(go_duration(45), "45s");
        assert_eq!(go_duration(63), "1m3s");
        assert_eq!(go_duration(3600), "1h0m0s");
        assert_eq!(go_duration(3723), "1h2m3s");
    }

    #[test]
    fn symbols_follow_gh() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(gh_symbol(&s("completed"), &s("success")), "✓");
        assert_eq!(gh_symbol(&s("success"), &None), "✓");
        assert_eq!(gh_symbol(&s("completed"), &s("skipped")), "-");
        assert_eq!(gh_symbol(&s("completed"), &s("cancelled")), "X");
        assert_eq!(gh_symbol(&s("failure"), &None), "X");
        assert_eq!(gh_symbol(&s("in_progress"), &None), "*");
        assert_eq!(gh_symbol(&s("queued"), &None), "*");
    }
}
