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
if [[ "$(uname -s)" == "Linux" ]]; then
  check_elf_runtime() {
    local binary="$1"
    local elf_metadata
    elf_metadata="$(readelf --program-headers --dynamic "$binary")"
    if grep -F '/nix/store/' <<<"$elf_metadata"; then
      echo "Linux release binary references a Nix runtime: $binary" >&2
      exit 1
    fi
  }
  check_elf_runtime "$build_target/release/lili"
  check_elf_runtime "$build_target/release/lili-hook"
  while IFS= read -r -d '' bundled_file; do
    if file --brief "$bundled_file" | grep -q '^ELF '; then
      check_elf_runtime "$bundled_file"
    fi
  done < <(find "$build_target/release/bundle" -type f -print0)
fi

release_parent="$workspace/release"
release_name="lili-$version-$platform"
release_root="$release_parent/$release_name"
rm -rf -- "$release_root"
mkdir -p \
  "$release_root/bin" \
  "$release_root/bundles" \
  "$release_root/docs" \
  "$release_root/examples" \
  "$release_root/pets/lili"

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
cp lili-pet/assets/fallback/pet.json lili-pet/assets/fallback/spritesheet.webp "$release_root/pets/lili/"
cp README.md "$release_root/"
cp docs/build.md docs/configuration.md docs/security-and-operations.md "$release_root/docs/"
cp examples/actions.toml "$release_root/examples/"
cp LICENSE NOTICE THIRD_PARTY_NOTICES.html "$release_root/"

signature_kind="platform-standard"
forwarder_signature_kind="platform-standard"
if [[ "$(uname -s)" == "Darwin" ]] && find "$release_root/bundles" -name '*.app' -type d -print -quit | grep -q .; then
  app_bundle="$(find "$release_root/bundles" -name '*.app' -type d -print -quit)"
  if codesign --verify --deep --strict "$app_bundle" >/dev/null 2>&1 \
    && codesign -dv --verbose=4 "$app_bundle" 2>&1 | grep -q '^Authority='; then
    signature_kind="signed"
  fi
  if codesign --verify --strict "$build_target/release/lili-hook" >/dev/null 2>&1 \
    && codesign -dv --verbose=4 "$build_target/release/lili-hook" 2>&1 | grep -q '^Authority='; then
    forwarder_signature_kind="signed"
  fi
fi
if [[ "${LILI_REQUIRE_SIGNED:-0}" == "1" \
  && ("$signature_kind" != "signed" || "$forwarder_signature_kind" != "signed") ]]; then
  echo "release signing was required but the app or hook forwarder is unsigned" >&2
  exit 1
fi

forwarder_root="$release_parent/forwarders/$platform"
rm -rf -- "$forwarder_root"
mkdir -p "$forwarder_root"
cp "$build_target/release/lili-hook" "$forwarder_root/lili-hook"
node scripts/write-forwarder-manifest.mjs \
  "$forwarder_root/lili-hook" \
  "$forwarder_root/manifest.json" \
  "$version" \
  "$platform" \
  "$forwarder_signature_kind"

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
printf '{"release":"%s","signatureKind":"%s","forwarder":"%s","forwarderSignatureKind":"%s"}\n' \
  "$archive" \
  "$signature_kind" \
  "$forwarder_root" \
  "$forwarder_signature_kind"
