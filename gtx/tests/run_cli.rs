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
    format!(r#"{{"total_count":{},"workflow_runs":[{}]}}"#, runs.len(), items.join(","))
}

#[test]
fn run_list_passes_filters_to_server() {
    let sha = "3a7312a85d77651aa15e943ed042c5dc8256313f";
    let server = FakeGitea::start(move |_| runs_json(&[(7, sha, "ci.yml@refs/heads/main")]));

    server
        .gtx()
        .args([
            "run", "list", "-R", "o/r", "--commit", sha, "--branch", "main", "--status",
            "completed", "--event", "push", "--user", "alice", "--limit", "5", "--json",
            "databaseId",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"databaseId\": 7"));

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
            "run", "list", "-R", "o/r", "--workflow", ".forgejo/workflows/ci.yml", "--json",
            "databaseId", "--jq", ".[].databaseId",
        ])
        .assert()
        .success()
        .stdout("1\n3\n");
}

/// `run list --json` takes gh's field names.
#[test]
fn run_list_json_uses_gh_field_names() {
    let server = FakeGitea::start(|_| runs_json(&[(7, "abc", ".gitea/workflows/ci.yml@refs/heads/main")]));
    let out = server
        .gtx()
        .args([
            "run", "list", "-R", "o/r", "--json",
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
        .args(["run", "watch", "-R", "o/r", "--commit", sha, "--exit-status"])
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
        .stderr(predicate::str::contains("No workflow runs found for commit deadbeef"));
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

const ARTIFACTS: &str = r#"{"total_count":1,"artifacts":[{"id":5,"name":"logs","size_in_bytes":3,"expired":false}]}"#;
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

    assert_eq!(std::fs::read_to_string(dir.path().join("logs/a.txt")).unwrap(), "x");
    assert_eq!(blob.auth(), vec![None]);
}
