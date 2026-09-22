use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let sha_marked = if dirty { format!("{sha}-dirty") } else { sha };

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    println!("cargo:rustc-env=GT_GIT_SHA={sha_marked}");
    println!("cargo:rustc-env=GT_BUILD_DATE={now}");

    // Re-run when HEAD moves or the index changes (i.e. on commit / staging).
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    println!("cargo:rerun-if-env-changed=GT_GIT_SHA");
    println!("cargo:rerun-if-env-changed=GT_BUILD_DATE");
}
