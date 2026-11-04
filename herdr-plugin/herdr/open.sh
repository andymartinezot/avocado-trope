#!/usr/bin/env bash
# Pane command: resolve the session of the pane this was opened from, and run
# `avocado-trope` on it. Every failure is printed here and held until you press enter,
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

. "$root/herdr/locate.sh"
command -v "$binary" >/dev/null 2>&1 || die "avocado-trope is not installed: see the Quick start in the README"

if ! target=$(bash "$root/herdr/resolve.sh" 2>&1); then
  die "$target"
fi

# The pane's agent is running, so ride the live edge; space and the scrubber
# go back.
"$binary" --provider "${target%% *}" --follow "${target#* }" \
  || die "avocado-trope exited with status $? (avocado-trope --provider ${target%% *} --follow ${target#* })"
