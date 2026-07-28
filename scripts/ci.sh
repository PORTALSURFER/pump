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
  --screenshots  Run the Radiant supported-size screenshot contract when the
                 plugin defines the `screenshot-test` cargo feature.
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
  if grep -qE '^[[:space:]]*vst3[[:space:]]*=' Cargo.toml; then
    : "${VST3_SDK_DIR:?VST3_SDK_DIR must be set when running with --vst3}"
    features=(--features vst3)
  else
    echo "[ci] vst3 feature not defined; skipping --vst3 checks"
    exit 0
  fi
fi

cargo fmt --all -- --check
if [[ ${#features[@]} -gt 0 ]]; then
  cargo clippy --all-targets "${features[@]}" -- -D warnings
  cargo test --all "${features[@]}"
else
  cargo clippy --all-targets -- -D warnings
  cargo test --all
fi

if [[ "${want_screenshots}" == "1" ]]; then
  if ! grep -qE '^[[:space:]]*screenshot-test[[:space:]]*=' Cargo.toml; then
    echo "[ci] screenshot-test feature not defined; skipping screenshot harness"
    exit 0
  fi

  rm -rf target/ui-screenshots
  mkdir -p target/ui-screenshots

  cargo test -r --features screenshot-test gui::screenshot_tests -- --nocapture

  required_captures=(
    target/ui-screenshots/pump/pump-min-720x540.png
    target/ui-screenshots/pump/pump-default-912x684.png
    target/ui-screenshots/pump/pump-max-1440x1080.png
    target/ui-screenshots/pump/pump-default-912x684-dpi-1_25.png
    target/ui-screenshots/pump/pump-components-states-720x360-1x.png
    target/ui-screenshots/pump/pump-components-states-720x360-2x.png
    target/ui-screenshots/pump/pump-bypass-active-912x684.png
    target/ui-screenshots/pump/pump-bypass-bypassed-912x684.png
    target/ui-screenshots/pump/pump-header-normal-912x684.png
    target/ui-screenshots/pump/pump-header-hovered-912x684.png
    target/ui-screenshots/pump/pump-header-copy-hovered-912x684.png
    target/ui-screenshots/pump/pump-header-a-hovered-912x684.png
    target/ui-screenshots/pump/pump-header-b-hovered-912x684.png
    target/ui-screenshots/pump/pump-header-pressed-912x684.png
    target/ui-screenshots/pump/pump-header-disabled-912x684.png
    target/ui-screenshots/pump/pump-header-a-active-912x684.png
    target/ui-screenshots/pump/pump-header-b-active-912x684.png
  )
  for capture in "${required_captures[@]}"; do
    if [[ ! -f "${capture}" ]]; then
      echo "[ci] required Pump screenshot was not produced: ${capture}" >&2
      exit 1
    fi
  done
fi
