use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Default `toybox.toml` emitted when the config file is missing.
const DEFAULT_TOYBOX_TOML: &str = r#"[artifacts]
clap = true
vst3 = true
target_dir = \"C:/dist\"
"#;

/// Runtime build configuration loaded from `toybox.toml`.
struct BuildConfig {
    clap: bool,
    vst3: bool,
    target_dir: PathBuf,
}

/// Artifact format selected for the current cargo invocation.
enum ArtifactKind {
    Clap,
    Vst3,
}

/// Resolved output locations for one artifact build.
struct ArtifactOutput {
    /// Concrete binary path passed to the linker `/OUT:` argument.
    link_output_path: PathBuf,
    /// User-facing artifact export path.
    export_path: PathBuf,
}

impl ArtifactKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
        }
    }
}

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
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let cargo_target_dir = cargo_target_dir(&manifest_dir);
    let output = output_paths_for(&artifact, &version, &profile, &cargo_target_dir, &config);

    create_parent(&output.link_output_path);
    println!(
        "cargo:rustc-cdylib-link-arg={}",
        windows_link_out_arg(&output.link_output_path)
    );
    println!(
        "cargo:warning=writing {} artifact to {}",
        artifact.label(),
        log_path(&output.export_path)
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

fn parse_config(contents: &str) -> Result<BuildConfig, String> {
    let mut clap = None;
    let mut vst3 = None;
    let mut target_dir = None;
    let mut in_artifacts = false;

    for raw_line in contents.lines() {
        let stripped = strip_inline_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            in_artifacts = line == "[artifacts]";
            continue;
        }
        if !in_artifacts {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match key {
            "clap" => clap = Some(parse_bool(value)?),
            "vst3" => vst3 = Some(parse_bool(value)?),
            "target_dir" => target_dir = Some(parse_string(value)?),
            _ => {}
        }
    }

    Ok(BuildConfig {
        clap: clap.unwrap_or(true),
        vst3: vst3.unwrap_or(true),
        target_dir: PathBuf::from(target_dir.unwrap_or_else(|| "C:/dist".to_string())),
    })
}

fn strip_inline_comment(line: &str) -> String {
    let mut in_double_quote = false;
    let mut result = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            '"' => {
                in_double_quote = !in_double_quote;
                result.push(ch);
            }
            '#' if !in_double_quote => break,
            _ => result.push(ch),
        }
    }
    result
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected bool, got `{value}`")),
    }
}

fn parse_string(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        Ok(trimmed[1..trimmed.len() - 1].to_string())
    } else {
        Err(format!("expected quoted string, got `{value}`"))
    }
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

fn output_paths_for(
    artifact: &ArtifactKind,
    version: &str,
    profile: &str,
    cargo_target_dir: &Path,
    config: &BuildConfig,
) -> ArtifactOutput {
    let output_root = if profile == "release" {
        config.target_dir.clone()
    } else {
        cargo_target_dir.join(profile)
    };

    match artifact {
        ArtifactKind::Clap => {
            let clap_path = output_root.join(format!("pump-v{version}.clap"));
            ArtifactOutput {
                link_output_path: clap_path.clone(),
                export_path: clap_path,
            }
        }
        ArtifactKind::Vst3 => {
            let bundle_name = format!("pump-v{version}-win.vst3");
            let bundle_root = output_root.join(&bundle_name);
            let binary_path = bundle_root
                .join("Contents")
                .join("x86_64-win")
                .join(&bundle_name);
            ArtifactOutput {
                link_output_path: binary_path,
                export_path: bundle_root,
            }
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
