#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(pwd -P)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${workspace_root}/target/coverage}"

lcov_path="${CARGO_TARGET_DIR}/result/lcov.info"
report_path="${CARGO_TARGET_DIR}/result/crap.md"
crap_threshold="${CRAP_THRESHOLD:-30}"

if [[ ! -s "${lcov_path}" ]]; then
  echo "Coverage report is missing: ${lcov_path}" >&2
  exit 1
fi

crap_args=(
  --workspace
  --lcov "${lcov_path}"
  --exclude "build.rs"
  --exclude "tests/**"
  --exclude "src/bin/lili-*-acceptance.rs"
  --exclude "src/bin/lili-codex-matrix.rs"
  --exclude "src/bin/lili-action-tree-fixture.rs"
  --exclude "src/acceptance_marketplace.rs"
  --exclude "src/desktop_acceptance.rs"
  --threshold "${crap_threshold}"
)

if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  cargo-crap "${crap_args[@]}" --format github
fi

cargo-crap "${crap_args[@]}" --format markdown --output "${report_path}"
cargo-crap "${crap_args[@]}" --fail-above --summary
