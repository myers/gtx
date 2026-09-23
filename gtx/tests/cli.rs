use assert_cmd::Command;
use predicates::prelude::*;

/// Test that gtx --help works
#[test]
fn test_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Gitea CLI"));
}

/// Test that gtx --version works
#[test]
fn test_version() {
    Command::cargo_bin("gtx")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("gtx "));
}

/// Test that gtx issue --help shows subcommands
#[test]
fn test_issue_help() {
    Command::cargo_bin("gtx")
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

/// Test that gtx pr --help shows subcommands
#[test]
fn test_pr_help() {
    Command::cargo_bin("gtx")
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

/// Test that gtx api --help shows flags
#[test]
fn test_api_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["api", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--method"))
        .stdout(predicate::str::contains("--jq"))
        .stdout(predicate::str::contains("--paginate"))
        .stdout(predicate::str::contains("--include"));
}

/// Test that gtx run --help shows subcommands
#[test]
fn test_run_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"))
        .stdout(predicate::str::contains("rerun"));
}

/// Test that gtx org --help shows subcommands
#[test]
fn test_org_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["org", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("view"));
}

/// Test that gtx label --help shows subcommands
#[test]
fn test_label_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["label", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("edit"))
        .stdout(predicate::str::contains("delete"));
}

/// Test that gtx milestone --help shows subcommands
#[test]
fn test_milestone_help() {
    Command::cargo_bin("gtx")
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

/// Test that gtx release --help shows subcommands
#[test]
fn test_release_help() {
    Command::cargo_bin("gtx")
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

/// Test that gtx project --help shows subcommands including column
#[test]
fn test_project_help() {
    Command::cargo_bin("gtx")
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

/// Test that gtx auth --help shows subcommands
#[test]
fn test_auth_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("logout"));
}

/// Test that gtx config --help shows subcommands
#[test]
fn test_config_help() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("set"))
        .stdout(predicate::str::contains("list"));
}

/// Test that gtx issue list without config gives a useful error
#[test]
fn test_issue_list_no_config() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["issue", "list", "-R", "owner/repo"])
        .env_remove("GITEA_URL")
        .env_remove("GITEA_TOKEN")
        .env_remove("GTX_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", "/tmp/gtx-test-nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("No Gitea URL configured"));
}

/// Test that gtx api without config gives a useful error
#[test]
fn test_api_no_config() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["api", "version"])
        .env_remove("GITEA_URL")
        .env_remove("GITEA_TOKEN")
        .env_remove("GTX_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", "/tmp/gtx-test-nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("No Gitea URL configured"));
}

/// Test that gtx completion generates output
#[test]
fn test_completion_bash() {
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_gtx()"));
}

/// Test that GTX_CONFIG points gtx at a config file outside ~/.config/gtx
#[test]
fn test_gtx_config_override() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("elsewhere.toml");
    Command::cargo_bin("gtx")
        .unwrap()
        .args(["config", "set", "default.url", "https://gitea.example.com"])
        .env("GTX_CONFIG", &file)
        .env("HOME", tmp.path().join("home"))
        .assert()
        .success();

    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("https://gitea.example.com"));
    assert!(!tmp.path().join("home").exists());
}

/// Test that the first run migrates ~/.config/gt to ~/.config/gtx
#[test]
fn test_migrates_old_config_dir() {
    let home = tempfile::tempdir().unwrap();
    let old = home.path().join(".config/gt");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("config.toml"), "[default]\nurl = \"https://old.example\"\n").unwrap();

    Command::cargo_bin("gtx")
        .unwrap()
        .args(["config", "get", "default.url"])
        .env_remove("GTX_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("https://old.example"))
        .stderr(predicate::str::contains("Migrated config"));

    assert!(home.path().join(".config/gtx/config.toml").is_file());
}
