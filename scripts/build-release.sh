#!/usr/bin/env bash
set -euo pipefail

workspace="$(pwd -P)"
version="$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "lili") | .version')"
build_target="$workspace/target/release-build"
export CARGO_TARGET_DIR="$build_target"
case "$(uname -s)" in
  Darwin)
    platform="$(uname -m)-apple-darwin"
    bundles="app,dmg"
    ;;
  Linux)
    platform="$(uname -m)-unknown-linux-gnu"
    bundles="deb,appimage"
    ;;
  *)
    echo "release assembly is unsupported on this host" >&2
    exit 2
    ;;
esac

cargo build --locked --release --package lili --features release-tools \
  --bin lili-hook --bin lili-codex-matrix
"$build_target/release/lili-codex-matrix" \
  "$build_target/release/lili-hook" \
  "$workspace/lili-session/tests/fixtures/codex"
rm -rf -- "$build_target/release/bundle"
cargo tauri build --bundles "$bundles" -- --locked

release_parent="$workspace/release"
release_name="lili-$version-$platform"
release_root="$release_parent/$release_name"
rm -rf -- "$release_root"
mkdir -p \
  "$release_root/bin" \
  "$release_root/bundles" \
  "$release_root/docs" \
  "$release_root/examples" \
  "$release_root/pet/lili"

cp "$build_target/release/lili" "$release_root/bin/"
cp "$build_target/release/lili-hook" "$release_root/bin/"
if [[ "$(uname -s)" == "Darwin" ]]; then
  mkdir -p "$release_root/bundles/macos" "$release_root/bundles/dmg"
  cp -R "$build_target/release/bundle/macos/Lili.app" "$release_root/bundles/macos/"
  find "$build_target/release/bundle/dmg" -maxdepth 1 -type f -name '*.dmg' \
    -exec cp {} "$release_root/bundles/dmg/" \;
else
  mkdir -p "$release_root/bundles/deb" "$release_root/bundles/appimage"
  find "$build_target/release/bundle/deb" -maxdepth 1 -type f -name '*.deb' \
    -exec cp {} "$release_root/bundles/deb/" \;
  find "$build_target/release/bundle/appimage" -maxdepth 1 -type f -name '*.AppImage' \
    -exec cp {} "$release_root/bundles/appimage/" \;
fi
cp -R dist "$release_root/web"
cp lili-pet/assets/fallback/pet.json lili-pet/assets/fallback/spritesheet.webp "$release_root/pet/lili/"
cp README.md "$release_root/"
cp docs/security-and-operations.md docs/toolchain.md "$release_root/docs/"
cp examples/actions.toml "$release_root/examples/"
cp LICENSE NOTICE THIRD_PARTY_NOTICES.html "$release_root/"

signature_kind="platform-standard"
if [[ "$(uname -s)" == "Darwin" ]] && find "$release_root/bundles" -name '*.app' -type d -print -quit | grep -q .; then
  app_bundle="$(find "$release_root/bundles" -name '*.app' -type d -print -quit)"
  if codesign --verify --deep --strict "$app_bundle" >/dev/null 2>&1 \
    && codesign -dv --verbose=4 "$app_bundle" 2>&1 | grep -q '^Authority='; then
    signature_kind="signed"
  fi
fi
if [[ "${LILI_REQUIRE_SIGNED:-0}" == "1" && "$signature_kind" != "signed" ]]; then
  echo "release signing was required but no platform identity was applied" >&2
  exit 1
fi

node scripts/release-manifest.mjs \
  "$release_root" \
  "$version" \
  "$platform" \
  "$signature_kind" \
  "$workspace"

mkdir -p "$release_parent"
archive="$release_parent/$release_name.tar.gz"
tar -C "$release_parent" -czf "$archive" "$release_name"
(
  cd "$release_parent"
  sha256sum "$release_name.tar.gz" > "$release_name.tar.gz.sha256"
)
printf '{"release":"%s","signatureKind":"%s"}\n' "$archive" "$signature_kind"
