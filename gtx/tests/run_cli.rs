mod common;

use assert_cmd::Command;
use predicates::prelude::*;

use common::FakeGitea;

fn runs_json(runs: &[(i64, &str, &str)]) -> String {
    let items: Vec<String> = runs
        .iter()
        .map(|(id, sha, path)| {
            format!(
                r#"{{"id":{id},"head_sha":"{sha}","path":"{path}","status":"completed","conclusion":"success","display_title":"t"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"total_count":{},"workflow_runs":[{}]}}"#,
        runs.len(),
        items.join(",")
    )
}

#[test]
fn run_list_passes_filters_to_server() {
    let sha = "3a7312a85d77651aa15e943ed042c5dc8256313f";
    let server = FakeGitea::start(move |_| runs_json(&[(7, sha, "ci.yml@refs/heads/main")]));

    server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "--commit",
            sha,
            "--branch",
            "main",
            "--status",
            "completed",
            "--event",
            "push",
            "--user",
            "alice",
            "--limit",
            "5",
            "--json",
            "databaseId",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"databaseId\":7"));

    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let q = &seen[0];
    assert!(q.starts_with("GET /api/v1/repos/o/r/actions/runs?"), "{q}");
    for want in [
        format!("head_sha={sha}"),
        "branch=main".into(),
        "status=completed".into(),
        "event=push".into(),
        "actor=alice".into(),
        "limit=5".into(),
    ] {
        assert!(q.contains(&want), "missing {want} in {q}");
    }
}

#[test]
fn run_list_workflow_filters_client_side() {
    let server = FakeGitea::start(|_| {
        runs_json(&[
            (1, "a", "ci.yml@refs/heads/main"),
            (2, "a", "release.yml@refs/heads/main"),
            (3, "b", "ci.yml@refs/heads/dev"),
        ])
    });

    server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "--workflow",
            ".forgejo/workflows/ci.yml",
            "--json",
            "databaseId",
            "--jq",
            ".[].databaseId",
        ])
        .assert()
        .success()
        .stdout("1\n3\n");
}

/// `run list --json` takes gh's field names.
#[test]
fn run_list_json_uses_gh_field_names() {
    let server =
        FakeGitea::start(|_| runs_json(&[(7, "abc", ".gitea/workflows/ci.yml@refs/heads/main")]));
    let out = server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "--json",
            "databaseId,headSha,status,conclusion,displayTitle,workflowName",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v,
        serde_json::json!([{
            "databaseId": 7, "headSha": "abc", "status": "completed",
            "conclusion": "success", "displayTitle": "t", "workflowName": "ci.yml",
        }])
    );

    server
        .gtx()
        .args(["run", "list", "-R", "o/r", "--json", "head_sha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown JSON field: \"head_sha\""));
}

#[test]
fn run_list_rejects_unknown_status() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["run", "list", "-R", "o/r", "--status", "bogus"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn run_watch_commit_watches_the_commits_runs() {
    let sha = "3a7312a85d77651aa15e943ed042c5dc8256313f";
    let server = FakeGitea::start(move |target| {
        if target.starts_with("/api/v1/repos/o/r/actions/runs?") {
            runs_json(&[(7, sha, "ci.yml@refs/heads/main")])
        } else if target.starts_with("/api/v1/repos/o/r/actions/runs/7/jobs") {
            r#"{"total_count":0,"jobs":[]}"#.into()
        } else if target.starts_with("/api/v1/repos/o/r/actions/runs/7") {
            r#"{"id":7,"status":"completed","conclusion":"failure","display_title":"t"}"#.into()
        } else {
            "{}".into()
        }
    });

    server
        .gtx()
        .args([
            "run",
            "watch",
            "-R",
            "o/r",
            "--commit",
            sha,
            "--exit-status",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Run #7"));

    let seen = server.seen();
    assert!(seen[0].contains(&format!("head_sha={sha}")), "{seen:?}");
}

#[test]
fn run_watch_commit_without_runs_errors() {
    let server = FakeGitea::start(|_| runs_json(&[]));

    server
        .gtx()
        .args(["run", "watch", "-R", "o/r", "--commit", "deadbeef"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "No workflow runs found for commit deadbeef",
        ));
}

#[test]
fn run_watch_id_and_commit_conflict() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["run", "watch", "-R", "o/r", "7", "--commit", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

fn zip_with(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buf);
    for (name, contents) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    buf.into_inner()
}

const ARTIFACTS: &str =
    r#"{"total_count":1,"artifacts":[{"id":5,"name":"logs","size_in_bytes":3,"expired":false}]}"#;
const ARTIFACT_META: &str = r#"{"id":5,"name":"logs","size_in_bytes":3}"#;

/// Gitea's `artifacts/{id}` is the metadata JSON; the bytes live at
/// `artifacts/{id}/zip`, which redirects to a signed blob URL. Like
/// `gh run download`, the zip is extracted into `<dir>/<artifact name>/`.
#[test]
fn run_download_fetches_zip_follows_redirect_and_extracts() {
    use common::Reply;
    let zip = zip_with(&[("out/result.txt", "hello")]);
    let server = FakeGitea::start_raw(move |_, target| match target {
        "/api/v1/repos/o/r/actions/runs/3/artifacts" => Reply::json(200, ARTIFACTS),
        "/api/v1/repos/o/r/actions/artifacts/5" => Reply::json(200, ARTIFACT_META),
        "/api/v1/repos/o/r/actions/artifacts/5/zip" => {
            Reply::redirect("/api/v1/repos/o/r/actions/artifacts/5/zip/raw?sig=x")
        }
        "/api/v1/repos/o/r/actions/artifacts/5/zip/raw?sig=x" => {
            Reply::bytes("application/zip", zip.clone())
        }
        _ => Reply::json(404, r#"{"message":"not found"}"#),
    });
    let dir = tempfile::tempdir().unwrap();

    server
        .gtx()
        .args(["run", "download", "-R", "o/r", "3", "-d"])
        .arg(dir.path())
        .assert()
        .success();

    let extracted = dir.path().join("logs/out/result.txt");
    assert_eq!(std::fs::read_to_string(&extracted).unwrap(), "hello");
    assert!(!dir.path().join("logs.zip").exists());
    let seen = server.seen();
    assert_eq!(
        seen.last().unwrap(),
        "GET /api/v1/repos/o/r/actions/artifacts/5/zip/raw?sig=x",
        "{seen:?}"
    );
    // Same-origin redirect keeps the token.
    assert_eq!(server.auth().last().unwrap().as_deref(), Some("token t"));
}

/// A body that is not a zip (e.g. an HTML login page or JSON) is an error,
/// not a successfully "downloaded" artifact.
#[test]
fn run_download_rejects_non_zip_body() {
    use common::Reply;
    let server = FakeGitea::start_raw(move |_, target| match target {
        "/api/v1/repos/o/r/actions/runs/3/artifacts" => Reply::json(200, ARTIFACTS),
        _ => Reply::json(200, ARTIFACT_META),
    });
    let dir = tempfile::tempdir().unwrap();

    server
        .gtx()
        .args(["run", "download", "-R", "o/r", "3", "-d"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a zip"));

    assert!(!dir.path().join("logs").exists());
    assert!(!dir.path().join("logs.zip").exists());
}

/// The token must not follow the zip redirect off to another host (e.g. a
/// presigned object-storage URL).
#[test]
fn run_download_withholds_token_from_foreign_redirect() {
    use common::Reply;
    let zip = zip_with(&[("a.txt", "x")]);
    let blob = FakeGitea::start_raw(move |_, _| Reply::bytes("application/zip", zip.clone()));
    let blob_url = format!("{}/bucket/5.zip?X-Amz-Signature=s", blob.url);
    let server = FakeGitea::start_raw(move |_, target| match target {
        "/api/v1/repos/o/r/actions/runs/3/artifacts" => Reply::json(200, ARTIFACTS),
        "/api/v1/repos/o/r/actions/artifacts/5/zip" => Reply::redirect(blob_url.clone()),
        _ => Reply::json(404, r#"{"message":"not found"}"#),
    });
    let dir = tempfile::tempdir().unwrap();

    server
        .gtx()
        .args(["run", "download", "-R", "o/r", "3", "-d"])
        .arg(dir.path())
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(dir.path().join("logs/a.txt")).unwrap(),
        "x"
    );
    assert_eq!(blob.auth(), vec![None]);
}

/// A queued run: Gitea reports `started_at`/`completed_at` as the Unix epoch
/// until a runner picks it up, but `created_at`/`updated_at` are real (#30).
const QUEUED_RUN: &str = r#"{"id":9,"status":"queued","display_title":"t","head_branch":"main",
    "created_at":"2020-05-06T07:08:09Z","updated_at":"2020-05-06T07:08:10Z",
    "started_at":"1970-01-01T00:00:00Z","completed_at":"1970-01-01T00:00:00Z"}"#;

#[test]
fn run_list_times_come_from_created_and_updated_at() {
    let server =
        FakeGitea::start(|_| format!(r#"{{"total_count":1,"workflow_runs":[{QUEUED_RUN}]}}"#));
    let out = server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "--json",
            "createdAt,updatedAt,startedAt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // gh prints an unset time as Go's zero time.
    assert_eq!(
        v,
        serde_json::json!([{
            "createdAt": "2020-05-06T07:08:09Z",
            "updatedAt": "2020-05-06T07:08:10Z",
            "startedAt": "0001-01-01T00:00:00Z",
        }])
    );

    server
        .gtx()
        .args(["run", "list", "-R", "o/r"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2020-05-06"))
        .stdout(predicate::str::contains("1970").not());
}

#[test]
fn run_view_jobs_unset_times_are_go_zero_time() {
    let server = FakeGitea::start(|target| {
        if target.starts_with("/api/v1/repos/o/r/actions/runs/9/jobs") {
            r#"{"total_count":1,"jobs":[{"id":1,"name":"build","status":"queued",
                "started_at":"1970-01-01T00:00:00Z","completed_at":"0001-01-01T00:00:00Z",
                "steps":[{"name":"s","number":1,"status":"queued",
                    "started_at":"1970-01-01T00:00:00Z"}]}]}"#
                .into()
        } else {
            QUEUED_RUN.into()
        }
    });
    server
        .gtx()
        .args([
            "run",
            "view",
            "9",
            "-R",
            "o/r",
            "--json",
            "createdAt,jobs",
            "--jq",
            "[.createdAt, .jobs[0].startedAt, .jobs[0].completedAt, .jobs[0].steps[0].startedAt, .jobs[0].steps[0].completedAt] | join(\" \")",
        ])
        .assert()
        .success()
        .stdout(
            "2020-05-06T07:08:09Z 0001-01-01T00:00:00Z 0001-01-01T00:00:00Z \
             0001-01-01T00:00:00Z 0001-01-01T00:00:00Z\n",
        );
}

/// A pull_request run as Gitea sends it: no `head_branch`, `path` on the PR
/// ref, the PR's head branch only under `pull_requests[0].head.ref`.
const PR_RUN: &str = r#"{"id":281,"status":"completed","conclusion":"failure",
    "display_title":"PR change","event":"pull_request","head_branch":null,
    "path":"walk.yml@refs/pull/13/head",
    "pull_requests":[{"id":6,"number":13,"url":"u",
        "head":{"ref":"feature","sha":"77f3","repo":{"id":5,"url":"u","name":"r"}},
        "base":{"ref":"main","sha":"8d4b","repo":{"id":5,"url":"u","name":"r"}}}]}"#;

/// gh's headBranch for a pull_request run is the PR's head branch.
#[test]
fn pull_request_run_branch_is_the_prs_head_branch() {
    let server = FakeGitea::start(|target| {
        if target.starts_with("/api/v1/repos/o/r/actions/runs?") {
            format!(r#"{{"total_count":1,"workflow_runs":[{PR_RUN}]}}"#)
        } else if target.starts_with("/api/v1/repos/o/r/actions/runs/281/jobs") {
            r#"{"total_count":0,"jobs":[]}"#.into()
        } else {
            PR_RUN.into()
        }
    });

    server
        .gtx()
        .args(["run", "list", "-R", "o/r", "--json", "headBranch"])
        .assert()
        .success()
        .stdout("[{\"headBranch\":\"feature\"}]\n");
    server
        .gtx()
        .args(["run", "list", "-R", "o/r"])
        .assert()
        .success()
        .stdout(predicate::str::contains("feature"));
    server
        .gtx()
        .args(["run", "view", "281", "-R", "o/r"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Branch: feature\n"));
    server
        .gtx()
        .args(["run", "view", "281", "-R", "o/r", "--json", "headBranch"])
        .assert()
        .success()
        .stdout("{\"headBranch\":\"feature\"}\n");
}

fn run_with_branch(id: i64, branch: &str) -> String {
    format!(
        r#"{{"id":{id},"status":"completed","display_title":"t","event":"push",
            "head_branch":"{branch}","path":"ci.yml@refs/heads/{branch}"}}"#
    )
}

fn pr_run(id: i64, head: &str) -> String {
    format!(
        r#"{{"id":{id},"status":"completed","display_title":"t","event":"pull_request",
            "path":"ci.yml@refs/pull/{id}/head","pull_requests":[{{"number":{id},
            "head":{{"ref":"{head}"}},"base":{{"ref":"main"}}}}]}}"#
    )
}

fn page(runs: &[String]) -> String {
    format!(
        r#"{{"total_count":{},"workflow_runs":[{}]}}"#,
        runs.len(),
        runs.join(",")
    )
}

/// Gitea's `branch` query matches `refs/heads/<branch>` only, never a
/// pull_request run's `refs/pull/N/head`, so `-b` also pages through
/// pull_request runs and matches their PR head branch client side, merging
/// both newest first.
#[test]
fn run_list_branch_finds_pull_request_runs() {
    let server = FakeGitea::start(|target| {
        let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let param = |k: &str| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix(k)?.strip_prefix('='))
                .and_then(|v| v.parse::<usize>().ok())
        };
        let (page_no, limit) = (param("page").unwrap_or(1), param("limit").unwrap_or(50));
        let all = if q.contains("branch=feature") {
            // Server-side branch match: push runs on the branch only.
            vec![
                run_with_branch(10, "feature"),
                run_with_branch(6, "feature"),
            ]
        } else if q.contains("event=pull_request") {
            // Every pull_request run, whatever its branch: the matches
            // span several pages.
            vec![
                pr_run(9, "other"),
                pr_run(8, "feature"),
                pr_run(7, "feature"),
                pr_run(5, "other"),
                pr_run(4, "other"),
                pr_run(3, "other"),
                pr_run(2, "feature"),
                pr_run(1, "feature"),
            ]
        } else {
            vec![]
        };
        let start = ((page_no - 1) * limit).min(all.len());
        page(&all[start..(start + limit).min(all.len())])
    });

    server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "-b",
            "feature",
            "-L",
            "4",
            "--json",
            "databaseId,headBranch",
            "--jq",
            ".[] | \"\\(.databaseId) \\(.headBranch)\"",
        ])
        .assert()
        .success()
        .stdout("10 feature\n8 feature\n7 feature\n6 feature\n");

    // Paging continues past pages with no match until the limit is met.
    server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "-b",
            "feature",
            "-L",
            "6",
            "--json",
            "databaseId",
            "--jq",
            ".[].databaseId",
        ])
        .assert()
        .success()
        .stdout("10\n8\n7\n6\n2\n1\n");

    // An event that can't be a pull_request run keeps the single server query.
    let before = server.seen().len();
    server
        .gtx()
        .args([
            "run",
            "list",
            "-R",
            "o/r",
            "-b",
            "feature",
            "-e",
            "push",
            "--json",
            "databaseId",
        ])
        .assert()
        .success();
    let seen = server.seen();
    assert_eq!(seen.len() - before, 1, "{:?}", &seen[before..]);
}
