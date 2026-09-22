use assert_cmd::Command;
use predicates::prelude::*;

/// Test that gt --help works
#[test]
fn test_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Gitea CLI"));
}

/// Test that gt --version works
#[test]
fn test_version() {
    Command::cargo_bin("gt")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("gt"));
}

/// Test that gt issue --help shows subcommands
#[test]
fn test_issue_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["issue", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("close"))
        .stdout(predicate::str::contains("reopen"))
        .stdout(predicate::str::contains("comment"));
}

/// Test that gt pr --help shows subcommands
#[test]
fn test_pr_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["pr", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("checkout"))
        .stdout(predicate::str::contains("merge"))
        .stdout(predicate::str::contains("close"))
        .stdout(predicate::str::contains("reopen"))
        .stdout(predicate::str::contains("comment"));
}

/// Test that gt api --help shows flags
#[test]
fn test_api_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["api", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--method"))
        .stdout(predicate::str::contains("--jq"))
        .stdout(predicate::str::contains("--paginate"))
        .stdout(predicate::str::contains("--include"));
}

/// Test that gt run --help shows subcommands
#[test]
fn test_run_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("rerun"));
}

/// Test that gt org --help shows subcommands
#[test]
fn test_org_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["org", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"));
}

/// Test that gt label --help shows subcommands
#[test]
fn test_label_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["label", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("edit"))
        .stdout(predicate::str::contains("delete"));
}

/// Test that gt milestone --help shows subcommands
#[test]
fn test_milestone_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["milestone", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("close"))
        .stdout(predicate::str::contains("reopen"));
}

/// Test that gt release --help shows subcommands
#[test]
fn test_release_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["release", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("download"))
        .stdout(predicate::str::contains("delete"));
}

/// Test that gt project --help shows subcommands including column
#[test]
fn test_project_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["project", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("close"))
        .stdout(predicate::str::contains("reopen"))
        .stdout(predicate::str::contains("column"));
}

/// Test that gt auth --help shows subcommands
#[test]
fn test_auth_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("logout"));
}

/// Test that gt config --help shows subcommands
#[test]
fn test_config_help() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("set"))
        .stdout(predicate::str::contains("list"));
}

/// Test that gt issue list without config gives a useful error
#[test]
fn test_issue_list_no_config() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["issue", "list", "-R", "owner/repo"])
        .env_remove("GITEA_URL")
        .env_remove("GITEA_TOKEN")
        .env("HOME", "/tmp/gt-test-nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("No Gitea URL configured"));
}

/// Test that gt api without config gives a useful error
#[test]
fn test_api_no_config() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["api", "version"])
        .env_remove("GITEA_URL")
        .env_remove("GITEA_TOKEN")
        .env("HOME", "/tmp/gt-test-nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("No Gitea URL configured"));
}

/// Test that gt completion generates output
#[test]
fn test_completion_bash() {
    Command::cargo_bin("gt")
        .unwrap()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gt"));
}
