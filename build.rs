use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use toybox::bundle::windows::{windows_bundle_paths, windows_rustc_link_arg, WindowsBundleFormat};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    emit_build_metadata(&manifest_dir);

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "linux".into());
    if target_os != "windows" {
        println!(
            "cargo:warning=skipping Windows VST3 bundle emission on non-Windows target ({target_os})"
        );
        return;
    }

    let version =
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION not set by cargo for build script");
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let paths = windows_bundle_paths(WindowsBundleFormat::Vst3, "Pump", &version);
    let output_path = paths.output_path(profile == "release");

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| {
            panic!(
                "failed to create output directory {}: {err}",
                parent.display()
            )
        });
    }
    println!(
        "cargo:rustc-cdylib-link-arg={}",
        windows_rustc_link_arg(output_path)
    );
    println!(
        "cargo:warning=writing Pump VST3 artifact to {}",
        output_path.display()
    );
}

fn emit_build_metadata(manifest_dir: &Path) {
    if let Some(git_dir) = resolved_git_dir(manifest_dir) {
        let head_path = git_dir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head) = fs::read_to_string(&head_path) {
            if let Some(reference) = head.strip_prefix("ref: ").map(str::trim) {
                println!(
                    "cargo:rerun-if-changed={}",
                    git_dir.join(reference).display()
                );
            }
        }
    }

    println!(
        "cargo:rustc-env=PUMP_BUILD_GIT_SHA_SHORT={}",
        git_short_sha(manifest_dir).unwrap_or_else(|| "unknown".to_string())
    );
}

fn resolved_git_dir(manifest_dir: &Path) -> Option<PathBuf> {
    let dot_git = manifest_dir.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }

    let contents = fs::read_to_string(&dot_git).ok()?;
    let path = contents.strip_prefix("gitdir: ")?.trim();
    let git_dir = PathBuf::from(path);
    Some(if git_dir.is_absolute() {
        git_dir
    } else {
        manifest_dir.join(git_dir)
    })
}

fn git_short_sha(manifest_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
