#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

lockfiles=(flake.lock Cargo.lock fuzz/Cargo.lock package-lock.json)
before="$(mktemp)"
after="$(mktemp)"
trap 'rm -f "$before" "$after"' EXIT

sha256sum "${lockfiles[@]}" >"$before"
"$@"
sha256sum "${lockfiles[@]}" >"$after"

if ! cmp -s "$before" "$after"; then
  echo "command modified a lockfile" >&2
  diff -u "$before" "$after" >&2 || true
  exit 1
fi
