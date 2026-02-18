#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

usage() {
  cat <<'EOF'
Usage:
  scripts/ci.sh [--vst3] [--screenshots]

Runs the same checks locally that CI enforces:
  - scripts/policy-check.sh (if present)
  - cargo fmt --check
  - cargo clippy -D warnings
  - cargo test
  - optional UI screenshot test (when supported)

Options:
  --vst3  Run checks with --features vst3 if the plugin defines a vst3 feature.
          Requires VST3_SDK_DIR to be set when the feature exists.
  --screenshots  Run `screenshot_renders_initial_ui` when the plugin supports the
                 screenshot harness (via a `screenshot-test` cargo feature).
EOF
}

want_vst3=0
want_screenshots=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --vst3) want_vst3=1; shift ;;
    --screenshots) want_screenshots=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -f scripts/policy-check.sh ]]; then
  bash scripts/policy-check.sh
fi

if [[ "${want_vst3}" == "1" && "${want_screenshots}" == "1" ]]; then
  echo "[ci] --vst3 and --screenshots are intentionally separate; run them as two invocations" >&2
  exit 2
fi

features=()
if [[ "${want_vst3}" == "1" ]]; then
  if grep -qE '^\\s*vst3\\s*=' Cargo.toml; then
    : "${VST3_SDK_DIR:?VST3_SDK_DIR must be set when running with --vst3}"
    features=(--features vst3)
  else
    echo "[ci] vst3 feature not defined; skipping --vst3 checks"
    exit 0
  fi
fi

cargo fmt --all -- --check
cargo clippy --all-targets "${features[@]}" -- -D warnings
cargo test --all "${features[@]}"

if [[ "${want_screenshots}" == "1" ]]; then
  if ! grep -qE '^[[:space:]]*screenshot-test[[:space:]]*=' Cargo.toml; then
    echo "[ci] screenshot-test feature not defined; skipping screenshot harness"
    exit 0
  fi

  rm -rf target/ui-screenshots
  mkdir -p target/ui-screenshots

  TOYBOX_UI_SCREENSHOT=1 \
    TOYBOX_UI_SCREENSHOT_DIR=target/ui-screenshots \
    cargo test -r --features screenshot-test screenshot_renders_initial_ui -- --nocapture

  if ! compgen -G "target/ui-screenshots/*/initial-ui-*.png" >/dev/null; then
    echo "[ci] screenshot-test feature is enabled but no initial-ui screenshots were produced under target/ui-screenshots" >&2
    exit 1
  fi
fi
