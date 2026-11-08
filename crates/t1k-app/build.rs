//! Bakes the build's git commit into the binary (`T1K_COMMIT`), so a bug report can say
//! exactly which code it came from. `T1K_COMMIT` in the environment wins (a release built
//! from a tarball has no git).

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-env-changed=T1K_COMMIT");
    println!("cargo:rerun-if-changed=build.rs");
    // Rebuild when HEAD moves (a checkout, or a commit: logs/HEAD grows with each one).
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
    let commit = std::env::var("T1K_COMMIT")
        .ok()
        .filter(|c| !c.trim().is_empty())
        .or_else(|| git(&["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=T1K_COMMIT={commit}");
}
