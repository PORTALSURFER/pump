#!/usr/bin/env sh
set -eu

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "${repo_root}"

fail() {
  echo "[policy] $*" >&2
  exit 1
}

manifests="$(git ls-files | grep -E '(^|/)Cargo.toml$' || true)"
[ -n "${manifests}" ] || fail "no Cargo.toml tracked by git"

for manifest in ${manifests}; do
  if grep -nE '^\[patch(\.|])' "${manifest}"; then
    fail "do not commit [patch] overrides in ${manifest} (use uncommitted .cargo/config.toml for local-only iteration)"
  fi

  if grep -nE '^toybox[[:space:]]*=[[:space:]]*[{][^}]*path[[:space:]]*=' "${manifest}"; then
    fail "toybox must not be a path dependency in ${manifest} (use a pinned git rev for reproducible builds)"
  fi

  if grep -nE '^toybox[[:space:]]*=[[:space:]]*[{][^}]*git[[:space:]]*=' "${manifest}" | grep -vqE 'rev[[:space:]]*='; then
    fail "toybox git dependency must be pinned with rev = \"...\" in ${manifest}"
  fi

  if grep -nE '^toybox[[:space:]]*=[[:space:]]*[{][^}]*git[[:space:]]*=' "${manifest}" \
    | grep -vqE 'rev[[:space:]]*=[[:space:]]*\"[0-9a-fA-F]{40}\"'; then
    fail "toybox rev must be a full 40-character SHA in ${manifest}"
  fi

  if grep -nE '^toybox[[:space:]]*=[[:space:]]*[{][^}]*git[[:space:]]*=' "${manifest}" \
    | grep -vqE 'git[[:space:]]*=[[:space:]]*\"https://github.com/PORTALSURFER/toybox[.]git\"'; then
    fail "toybox git URL must be https://github.com/PORTALSURFER/toybox.git in ${manifest} (normalize .git suffix so local [patch] works consistently)"
  fi

  if grep -nE 'branch[[:space:]]*=' "${manifest}"; then
    fail "branch pins are not allowed in ${manifest} (use rev)"
  fi

  if grep -nE '^(clack-(plugin|extensions|common|host)|clap-sys|baseview)[[:space:]]*=' "${manifest}"; then
    fail "plugins must not depend directly on CLAP/baseview crates; use toybox re-exports instead (${manifest})"
  fi
done

if [ -f .cargo/config.toml ]; then
  if git ls-files --error-unmatch .cargo/config.toml >/dev/null 2>&1; then
    fail "do not commit .cargo/config.toml (local-only patch workflow)"
  fi
fi

plugin_name="$(basename "${repo_root}")"
coverage_file="${TOYBOX_SCREENSHOT_COVERAGE_FILE:-../scripts/screenshot-coverage.toml}"

coverage_supported_for() {
  # Outputs: true | false | (empty)
  plugin="$1"
  file="$2"
  [ -f "${file}" ] || return 0

  # Find the plugin section and print the first supported= line value.
  sed -n "
    /^[[:space:]]*\\[plugins\\.${plugin}\\][[:space:]]*$/,/^[[:space:]]*\\[plugins\\./{
      /^[[:space:]]*supported[[:space:]]*=/{
        s/^[[:space:]]*supported[[:space:]]*=[[:space:]]*\\(true\\|false\\).*/\\1/p
        q
      }
    }
  " "${file}"
}

has_screenshot_symbol_in_src() {
  src_files="$(git ls-files | grep -E '^src/.*[.]rs$' || true)"
  [ -n "${src_files}" ] || return 1
  # shellcheck disable=SC2086
  echo "${src_files}" | xargs grep -n 'screenshot_renders_initial_ui' >/dev/null 2>&1
}

# Screenshot coverage checks (meta-workspace only):
# If this plugin has a src/gui.rs, then either:
# - it provides screenshot coverage via screenshot-test + screenshot_renders_initial_ui, or
# - it is explicitly marked unsupported in the meta-root coverage policy file.
#
# This check is skipped when the coverage file is not available (for example:
# when running inside the standalone plugin repo outside the meta workspace).
if git ls-files --error-unmatch src/gui.rs >/dev/null 2>&1; then
  has_screenshot_feature=0
  if grep -qE '^[[:space:]]*screenshot-test[[:space:]]*=' Cargo.toml; then
    has_screenshot_feature=1
  fi

  has_screenshot_symbol=0
  if has_screenshot_symbol_in_src; then
    has_screenshot_symbol=1
  fi

  supported="$(coverage_supported_for "${plugin_name}" "${coverage_file}")"

  if [ "${has_screenshot_feature}" -eq 0 ] || [ "${has_screenshot_symbol}" -eq 0 ]; then
    if [ -f "${coverage_file}" ]; then
      if [ "${supported}" = "false" ]; then
        : # explicitly unsupported; ok
      elif [ "${supported}" = "true" ]; then
        fail "${plugin_name} has src/gui.rs but does not provide screenshot_renders_initial_ui with screenshot-test; coverage policy marks it supported (${coverage_file})"
      else
        fail "${plugin_name} has src/gui.rs but has no screenshot coverage and no coverage policy entry in ${coverage_file}"
      fi
    fi
  else
    if [ -f "${coverage_file}" ] && [ "${supported}" = "false" ]; then
      fail "${plugin_name} provides screenshot coverage but is marked supported=false in ${coverage_file}; update the policy file"
    fi
  fi
fi

# GUI drift checks: plugins should not vendor Patchbay internals or reference the
# Patchbay crate directly. GUI mechanics belong in `toybox/patchbay-gui`.
drift_paths="$(git ls-files | grep -E '(^|/)(patchbay-gui|patchbay_gui)/' || true)"
if [ -n "${drift_paths}" ]; then
  echo "${drift_paths}" >&2
  fail "do not vendor patchbay-gui into plugins; use toybox re-exports and framework-owned GUI mechanics"
fi

source_files="$(git ls-files | grep -E '^src/.*[.]rs$' || true)"
if [ -n "${source_files}" ]; then
  # shellcheck disable=SC2086
  violations="$(echo "${source_files}" | xargs grep -nE 'patchbay_gui::' 2>/dev/null | grep -v 'toybox::patchbay_gui::' || true)"
  if [ -n "${violations}" ]; then
    echo "${violations}" >&2
    fail "do not reference patchbay_gui directly; use toybox re-exports (toybox::patchbay_gui::...)"
  fi
fi

# Slot grammar drift checks:
# - legacy section helper API is disallowed
# - canonical slot grids must not use pixel tracks
if [ -n "${source_files}" ]; then
  # shellcheck disable=SC2086
  legacy_slot_api="$(echo "${source_files}" | xargs grep -nE 'row_sections|column_sections|weighted_section|fraction_section|fill_section|weighted_section_lengths' 2>/dev/null || true)"
  if [ -n "${legacy_slot_api}" ]; then
    echo "${legacy_slot_api}" >&2
    fail "legacy section helper API is not allowed; use slot helpers (row_slots/column_slots/weighted_slot/fraction_slot/fill_slot/weighted_slot_lengths)"
  fi

  # shellcheck disable=SC2086
  slot_kind_refs="$(echo "${source_files}" | xargs grep -nE 'GridKind::Slot(Row|Column)' 2>/dev/null || true)"
  if [ -n "${slot_kind_refs}" ]; then
    # shellcheck disable=SC2086
    slot_px_tracks="$(echo "${source_files}" | xargs grep -nE 'TrackSize::Px[[:space:]]*[(]' 2>/dev/null || true)"
    if [ -n "${slot_px_tracks}" ]; then
      echo "${slot_px_tracks}" >&2
      fail "slot grids must use fraction/fill tracks only; remove TrackSize::Px from slot-grid definitions"
    fi
  fi
fi

# Strict slot-tree invariant checks are required for plugins that author UIs
# with slot-layout helpers.
if git ls-files --error-unmatch src/gui.rs >/dev/null 2>&1; then
  strict_slot_helpers="$(grep -nE 'row_slots|column_slots|weighted_slot|fraction_slot|fill_slot' src/gui.rs || true)"
  if [ -n "${strict_slot_helpers}" ]; then
    if ! grep -q 'fn emitted_ui_spec_passes_strict_slot_validation' src/gui.rs; then
      fail "src/gui.rs uses strict slot helpers and must include emitted_ui_spec_passes_strict_slot_validation to guard root->slot->container/widget invariants"
    fi
  fi
fi

echo "[policy] ok"
