//! `gtx run` against a tiny in-process fake Gitea, so we can assert on the
//! query string gtx sends and on how it treats the response.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
use predicates::prelude::*;

/// Fake server: answers every request with `route(path_and_query)` as a JSON
/// body and records the request targets it saw.
struct FakeGitea {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeGitea {
    fn start(route: impl Fn(&str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                }
                let target = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = route(&target);
                seen_thread.lock().unwrap().push(target);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        FakeGitea { url, seen }
    }

    fn gtx(&self) -> Command {
        let mut cmd = Command::cargo_bin("gtx").unwrap();
        cmd.env("GITEA_URL", &self.url)
            .env("GITEA_TOKEN", "t")
            .env_remove("GITEA_SERVER")
            .env("GTX_CONFIG", "/nonexistent/gtx-test.toml");
        cmd
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

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
            "completed", "--event", "push", "--user", "alice", "--limit", "5", "--json", "id",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 7"));

    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let q = &seen[0];
    assert!(q.starts_with("/api/v1/repos/o/r/actions/runs?"), "{q}");
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
            "run", "list", "-R", "o/r", "--workflow", ".forgejo/workflows/ci.yml", "--json", "id",
            "--jq", ".[].id",
        ])
        .assert()
        .success()
        .stdout("1\n3\n");
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
