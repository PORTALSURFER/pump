use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "src/build_support.rs"]
mod build_support;

use build_support::{output_path_for, parse_config, ArtifactKind, BuildConfig};

/// Default `toybox.toml` emitted when the config file is missing.
const DEFAULT_TOYBOX_TOML: &str = r#"[artifacts]
clap = true
vst3 = true
target_dir = \"C:/dist\"
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let config_path = manifest_dir.join("toybox.toml");
    println!("cargo:rerun-if-changed={}", config_path.display());

    let config = load_or_create_config(&config_path);
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "linux".into());

    if target_os != "windows" {
        println!(
            "cargo:warning=skipping artifact bundle emission on non-Windows target ({target_os})"
        );
        return;
    }

    let artifact = select_artifact_for_invocation(&config);
    let version =
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION not set by cargo for build script");
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let cargo_target_dir = cargo_target_dir(&manifest_dir);
    let output_path = output_path_for(artifact, &version, &profile, &cargo_target_dir, &config);

    create_parent(&output_path);
    println!(
        "cargo:rustc-cdylib-link-arg={}",
        windows_link_out_arg(&output_path)
    );
    println!(
        "cargo:warning=writing {} artifact to {}",
        artifact.label(),
        log_path(&output_path)
    );
}

fn cargo_target_dir(manifest_dir: &Path) -> PathBuf {
    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        PathBuf::from(dir)
    } else {
        manifest_dir.parent().unwrap_or(manifest_dir).join("target")
    }
}

fn load_or_create_config(path: &Path) -> BuildConfig {
    if !path.exists() {
        fs::write(path, DEFAULT_TOYBOX_TOML).unwrap_or_else(|err| {
            panic!("failed to create default config {}: {err}", path.display())
        });
    }

    let contents = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read config {}: {err}", path.display()));
    parse_config(&contents).unwrap_or_else(|err| panic!("invalid config {}: {err}", path.display()))
}

fn select_artifact_for_invocation(config: &BuildConfig) -> ArtifactKind {
    if let Some(active) = env::var_os("TOYBOX_ACTIVE_ARTIFACT") {
        let active = active.to_string_lossy().to_ascii_lowercase();
        return match active.as_str() {
            "clap" => ArtifactKind::Clap,
            "vst3" => ArtifactKind::Vst3,
            other => {
                panic!("unsupported TOYBOX_ACTIVE_ARTIFACT `{other}`. Expected `clap` or `vst3`.");
            }
        };
    }

    match (config.clap, config.vst3) {
        (true, false) => ArtifactKind::Clap,
        (false, true) => ArtifactKind::Vst3,
        (true, true) => {
            println!(
                "cargo:warning=toybox.toml enables both artifacts and TOYBOX_ACTIVE_ARTIFACT is unset; defaulting to vst3 for this cargo invocation. Use scripts/build-artifacts.ps1 (or scripts/build-artifacts.sh) to emit both."
            );
            ArtifactKind::Vst3
        }
        (false, false) => {
            panic!("toybox.toml must enable at least one artifact (`clap` or `vst3`).");
        }
    }
}

fn log_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn windows_link_out_arg(path: &Path) -> String {
    // `cargo:rustc-cdylib-link-arg` forwards this value directly to `link.exe`.
    // Quoting here can become part of the literal argument and break path parsing.
    format!("/OUT:{}", path.display())
}

fn create_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| {
            panic!(
                "failed to create output directory {}: {err}",
                parent.display()
            )
        });
    }
}
