use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn admin_and_org_are_mutually_exclusive() {
    let mut cmd = Command::cargo_bin("gt").unwrap();
    cmd.args(["runner", "list", "--admin", "--org", "acme"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}
