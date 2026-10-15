use std::{env, path::Path, process::Command};

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn main() {
    let directory = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let directory = Path::new(&directory);
    println!("cargo:rerun-if-changed=build.rs");
    // Linked worktrees have their own HEAD outside the source directory. Watch
    // it so switching branches at the same commit still refreshes the label.
    if let Some(head) = git(
        directory,
        &["rev-parse", "--path-format=absolute", "--git-path", "HEAD"],
    ) {
        println!("cargo:rerun-if-changed={head}");
    }
    let branch = git(directory, &["symbolic-ref", "--quiet", "HEAD"])
        .and_then(|reference| reference.strip_prefix("refs/heads/").map(str::to_owned))
        .unwrap_or_else(|| {
            if git(directory, &["rev-parse", "--verify", "HEAD"]).is_some() {
                "detached".into()
            } else {
                "unknown".into()
            }
        });
    println!("cargo:rustc-env=KESTREL_BRANCH={branch}");
}
