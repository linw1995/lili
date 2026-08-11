#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(pwd -P)"
coverage_root="${workspace_root}/target/coverage"
test_tmp="$(mktemp -d /tmp/lili-coverage.XXXXXX)"

trap 'rm -rf -- "${test_tmp}"' EXIT

export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="${coverage_root}"
export LLVM_PROFILE_FILE="${coverage_root}/data/lili-%p-%m.profraw"
coverage_rustflags="-Cinstrument-coverage -Ccodegen-units=1 -Copt-level=0"
if [[ "$(uname -s)" != "Darwin" ]]; then
  coverage_rustflags+=" -Clink-dead-code"
fi
export RUSTFLAGS="${coverage_rustflags}"
export TMPDIR="${test_tmp}"

rm -rf -- "${coverage_root}/data" "${coverage_root}/result"
mkdir -p "${coverage_root}/data" "${coverage_root}/result"

cargo test --locked --workspace --all-targets --features lili/acceptance "$@"

grcov "${coverage_root}/data" \
  --llvm \
  --branch \
  --source-dir "${workspace_root}" \
  --ignore-not-existing \
  --ignore '../*' \
  --ignore '/*' \
  --binary-path "${coverage_root}/debug/deps" \
  --output-types html,cobertura,lcov,markdown \
  --output-path "${coverage_root}/result"

cp "${coverage_root}/result/lcov" "${coverage_root}/result/lcov.info"
tail -n 1 "${coverage_root}/result/markdown.md"
