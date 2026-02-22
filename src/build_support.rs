//! Shared build-script parsing and output-path helpers.
//!
//! This module is consumed by `build.rs` and unit-tested from the crate test
//! harness to catch artifact path regressions.

use std::path::{Path, PathBuf};

/// Runtime build configuration loaded from `toybox.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildConfig {
    /// Whether CLAP artifact emission is enabled.
    pub clap: bool,
    /// Whether VST3 artifact emission is enabled.
    pub vst3: bool,
    /// Release output directory for emitted artifacts.
    pub target_dir: PathBuf,
}

/// Artifact format selected for the current cargo invocation.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ArtifactKind {
    /// CLAP plugin bundle.
    Clap,
    /// VST3 plugin bundle.
    Vst3,
}

impl ArtifactKind {
    /// User-facing label for build log output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
        }
    }
}

/// Parse `toybox.toml` artifact settings.
pub fn parse_config(contents: &str) -> Result<BuildConfig, String> {
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

/// Resolve final artifact output path for one cargo invocation.
pub fn output_path_for(
    artifact: ArtifactKind,
    version: &str,
    profile: &str,
    cargo_target_dir: &Path,
    config: &BuildConfig,
) -> PathBuf {
    let output_root = if profile == "release" {
        config.target_dir.clone()
    } else {
        cargo_target_dir.join(profile)
    };

    match artifact {
        ArtifactKind::Clap => output_root.join(format!("pump-v{version}-win.clap")),
        ArtifactKind::Vst3 => output_root.join(format!("pump-v{version}-win.vst3")),
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{output_path_for, parse_config, ArtifactKind, BuildConfig};

    #[test]
    fn parse_config_reads_artifacts_section_and_keeps_defaults() {
        let config = parse_config(
            r#"
[ignored]
clap = false

[artifacts]
target_dir = "C:/dist" # keep this comment ignored
"#,
        )
        .expect("config should parse");

        assert_eq!(
            config,
            BuildConfig {
                clap: true,
                vst3: true,
                target_dir: PathBuf::from("C:/dist"),
            }
        );
    }

    #[test]
    fn parse_config_rejects_invalid_bool_and_unquoted_path() {
        assert!(parse_config("[artifacts]\nclap = maybe\n").is_err());
        assert!(parse_config("[artifacts]\ntarget_dir = C:/dist\n").is_err());
    }

    #[test]
    fn output_path_for_uses_windows_suffix_and_extension() {
        let config = BuildConfig {
            clap: true,
            vst3: true,
            target_dir: PathBuf::from("C:/dist"),
        };
        let cargo_target = PathBuf::from("target");
        let clap_path = output_path_for(
            ArtifactKind::Clap,
            "0.2.0",
            "release",
            &cargo_target,
            &config,
        );
        let vst3_path = output_path_for(
            ArtifactKind::Vst3,
            "0.2.0",
            "release",
            &cargo_target,
            &config,
        );

        assert_eq!(
            clap_path,
            PathBuf::from("C:/dist").join("pump-v0.2.0-win.clap")
        );
        assert_eq!(
            vst3_path,
            PathBuf::from("C:/dist").join("pump-v0.2.0-win.vst3")
        );
    }

    #[test]
    fn output_path_for_debug_uses_cargo_target_profile_dir() {
        let config = BuildConfig {
            clap: true,
            vst3: true,
            target_dir: PathBuf::from("C:/dist"),
        };
        let cargo_target = PathBuf::from("/tmp/work/target");
        let output = output_path_for(ArtifactKind::Vst3, "0.2.0", "debug", &cargo_target, &config);

        assert_eq!(
            output,
            PathBuf::from("/tmp/work/target/debug").join("pump-v0.2.0-win.vst3")
        );
    }

    #[test]
    fn artifact_kind_label_matches_expected_names() {
        assert_eq!(ArtifactKind::Clap.label(), "CLAP");
        assert_eq!(ArtifactKind::Vst3.label(), "VST3");
    }
}
