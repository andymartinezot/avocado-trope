#!/bin/sh
# Install the latest avocado-trope release into ~/.local/bin (or $INSTALL_DIR).
#
#   curl -fsSL https://raw.githubusercontent.com/andymartinezot/avocado-trope/main/scripts/install.sh | sh
#
# Checks the download against the release's SHA256SUMS and installs nothing if
# they differ.
set -eu

repo="andymartinezot/avocado-trope"
dir="${INSTALL_DIR:-$HOME/.local/bin}"
base="https://github.com/$repo/releases/latest/download"

fail() {
  echo "avocado-trope: $1" >&2
  exit 1
}

command -v curl >/dev/null 2>&1 || fail "needs curl"

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64 | Darwin/aarch64) target="darwin_arm64" ;;
  Darwin/x86_64 | Darwin/amd64) target="darwin_amd64" ;;
  Linux/aarch64 | Linux/arm64) target="linux_arm64" ;;
  Linux/x86_64 | Linux/amd64) target="linux_amd64" ;;
  *) fail "no prebuilt binary for $(uname -s)/$(uname -m); build from source instead" ;;
esac

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    fail "needs sha256sum or shasum"
  fi
}

asset="avocado-trope_${target}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || fail "cannot reach the latest release"
curl -fsSL -o "$tmp/$asset" "$base/$asset" || fail "the latest release has no $asset"

expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a {print tolower($1); exit}' "$tmp/SHA256SUMS")
[ -n "$expected" ] || fail "the release lists no checksum for $asset"
[ "$(sha256_of "$tmp/$asset")" = "$expected" ] || fail "the download is damaged (checksum mismatch)"

mkdir -p "$dir"
chmod +x "$tmp/$asset"
mv -f "$tmp/$asset" "$dir/avocado-trope"
echo "avocado-trope: installed $("$dir/avocado-trope" --version) into $dir."
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH to run it from anywhere." ;;
esac
