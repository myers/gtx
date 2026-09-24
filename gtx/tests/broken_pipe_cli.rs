//! A closed stdout (`gtx ... | head -1`) ends gtx the way it ends gh and
//! other Unix tools: killed quietly by SIGPIPE, never a panic.

#![cfg(unix)]

mod common;

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

use common::FakeGitea;

const RUNS: &str = r#"{"total_count":1,"workflow_runs":[{"id":7,"head_sha":"3a7312a85d77651aa15e943ed042c5dc8256313f","path":"ci.yml@refs/heads/main","status":"completed","conclusion":"success","display_title":"t"}]}"#;

/// Run gtx with its stdout already closed; return (status, stderr).
fn run_with_closed_stdout(server: &FakeGitea, args: &[&str]) -> (std::process::ExitStatus, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gtx"))
        .args(args)
        .env("GITEA_URL", &server.url)
        .env("GITEA_TOKEN", "t")
        .env_remove("GITEA_SERVER")
        .env("GTX_CONFIG", "/nonexistent/gtx-test.toml")
        .env_remove("RUST_BACKTRACE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    (
        out.status,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn closed_stdout_exits_quietly_via_sigpipe() {
    let server = FakeGitea::start(|_| RUNS.into());
    for args in [
        &["run", "list", "-R", "o/r"][..],
        &["run", "list", "-R", "o/r", "--json", "databaseId"],
        &[
            "run",
            "list",
            "-R",
            "o/r",
            "--json",
            "databaseId",
            "-q",
            ".[]",
        ],
        &[
            "run",
            "list",
            "-R",
            "o/r",
            "--json",
            "databaseId",
            "-t",
            "{{range .}}{{.databaseId}}{{end}}",
        ],
        &["api", "repos/o/r/actions/runs"],
    ] {
        let (status, stderr) = run_with_closed_stdout(&server, args);
        assert!(stderr.is_empty(), "{args:?}: unexpected stderr: {stderr}");
        assert_eq!(
            status.signal(),
            Some(13),
            "{args:?}: want SIGPIPE, got {status:?}"
        );
    }
}
