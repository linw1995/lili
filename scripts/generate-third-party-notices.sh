#!/usr/bin/env bash

set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
output_path="${1:-${workspace_root}/THIRD_PARTY_NOTICES.html}"
raw_output="$(mktemp "${TMPDIR:-/tmp}/lili-third-party-notices.XXXXXX")"

trap 'rm -f "${raw_output}"' EXIT

cd "${workspace_root}"

cargo-about generate \
  --all-features \
  --fail \
  --locked \
  --offline \
  --output-file "${raw_output}" \
  --workspace \
  about.hbs

LC_ALL=C awk '{ sub(/\r$/, ""); sub(/[[:space:]]+$/, ""); print }' \
  "${raw_output}" > "${output_path}"
