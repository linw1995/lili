#!/usr/bin/env bash

set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
generated_notices="$(mktemp "${TMPDIR:-/tmp}/lili-third-party-notices-check.XXXXXX")"

trap 'rm -f "${generated_notices}"' EXIT

bash "${workspace_root}/scripts/generate-third-party-notices.sh" "${generated_notices}"

test -s "${generated_notices}"
