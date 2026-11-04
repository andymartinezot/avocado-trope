#!/bin/sh
# Install the avocado-trope binary into the plugin's bin/.
#
# Downloads the release matching this plugin's version and checks it against
# the release's SHA256SUMS. If that is not possible (unsupported platform, no
# release yet, network or checksum failure), builds from source with cargo.
set -u

repo="andymartinezot/avocado-trope"
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
out="$root/bin/avocado-trope"
base_url="https://github.com/$repo/releases/download"

cleanup() { [ -n "${tmpdir:-}" ] && rm -rf "$tmpdir"; }
trap cleanup EXIT

build_from_source() {
  cleanup
  if ! command -v cargo >/dev/null 2>&1; then
    echo "avocado-trope: no matching prebuilt binary and Rust is not installed (https://rustup.rs)." >&2
    exit 1
  fi
  exec cargo install --locked --root "$root" --git "https://github.com/$repo" --tag "v${version:-0.0.0}" avocado-trope
}

fallback() {
  echo "avocado-trope: $1; building from source instead." >&2
  build_from_source
}

download() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$2" "$base_url/v$version/$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$base_url/v$version/$1"
  else
    return 127
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    return 127
  fi
}

version=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$root/herdr-plugin.toml" | head -n 1)
[ -n "$version" ] || fallback "could not read the version from herdr-plugin.toml"

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64 | Darwin/aarch64) target="darwin_arm64" ;;
  Darwin/x86_64 | Darwin/amd64) target="darwin_amd64" ;;
  Linux/aarch64 | Linux/arm64) target="linux_arm64" ;;
  Linux/x86_64 | Linux/amd64) target="linux_amd64" ;;
  *) fallback "no prebuilt binary for $(uname -s)/$(uname -m)" ;;
esac

asset="avocado-trope_${target}"
tmpdir=$(mktemp -d 2>/dev/null) || fallback "could not create a temporary directory"
download SHA256SUMS "$tmpdir/SHA256SUMS" || fallback "no release v$version"
download "$asset" "$tmpdir/$asset" || fallback "no $asset in release v$version"

expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a {print tolower($1); exit}' "$tmpdir/SHA256SUMS")
[ -n "$expected" ] || fallback "release v$version lists no checksum for $asset"
actual=$(sha256_of "$tmpdir/$asset") || fallback "no SHA-256 tool available"
[ "$actual" = "$expected" ] || fallback "checksum mismatch for $asset"

mkdir -p "$root/bin" && chmod +x "$tmpdir/$asset" && mv -f "$tmpdir/$asset" "$out" ||
  fallback "could not install the downloaded binary"
echo "avocado-trope: installed verified v$version ($target)."
