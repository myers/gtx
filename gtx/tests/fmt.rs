//! Guard: the workspace must stay rustfmt-clean (#15). There's no CI, so
//! `cargo test` is where this gets checked. Fix failures with `cargo fmt --all`.

use std::path::Path;
use std::process::Command;

#[test]
fn workspace_is_rustfmt_clean() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let have_rustfmt = Command::new(&cargo)
        .args(["fmt", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !have_rustfmt {
        eprintln!("skipping: rustfmt not installed");
        return;
    }
    let out = Command::new(&cargo)
        .args(["fmt", "--all", "--check"])
        .current_dir(root)
        .output()
        .expect("run cargo fmt --check");
    assert!(
        out.status.success(),
        "workspace is not rustfmt-clean; run `cargo fmt --all`:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
