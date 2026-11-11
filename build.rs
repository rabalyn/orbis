use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(path) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(path) = git(&["rev-parse", "--git-path", "refs/tags"]) {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(path) = git(&["rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={path}");
    }

    let build = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let tag = git(&[
        "describe",
        "--tags",
        "--exact-match",
        "--match",
        "v[0-9]*",
        "HEAD",
    ])
    .unwrap_or_default();
    let generated =
        format!("pub const BUILD_ID: &str = {build:?};\npub const RELEASE_TAG: &str = {tag:?};\n");
    let out =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR")).join("build_info.rs");
    fs::write(out, generated).expect("write build metadata");
}
