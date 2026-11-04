#!/usr/bin/env bash
# Pane command: resolve the session of the pane this was opened from, and run
# `zoe` on it. Every failure is printed here and held until you press enter,
# since this terminal is what the user is looking at.
set -euo pipefail

# Herdr sets HERDR_PLUGIN_ROOT; run by hand, the plugin root is this
# script's parent directory, never the current directory.
root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"

die() {
  printf '\n  %s\n\n' "$*" >&2
  read -r -p '  press enter to close ' _ || true
  exit 1
}

# Linked from a checkout of this repo: run that checkout's own release build
# (`cargo build --release`), so local changes show up without installing.
zoe=zoe
if [ -x "$root/../target/release/zoe" ]; then
  zoe="$root/../target/release/zoe"
fi
command -v "$zoe" >/dev/null 2>&1 || die "zoe is not on PATH (cargo build --release in this repo, or cargo install --path .)"

if ! target=$(bash "$root/herdr/resolve.sh" 2>&1); then
  die "$target"
fi

# The pane's agent is running, so ride the live edge; space and the scrubber
# go back.
"$zoe" --provider "${target%% *}" --follow "${target#* }" \
  || die "zoe exited with status $? (zoe --provider ${target%% *} --follow ${target#* })"
