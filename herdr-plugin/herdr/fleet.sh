#!/usr/bin/env bash
# Pane command: every agent Herdr knows about, on one screen. Failures are held
# until you press enter, since this terminal is what the user is looking at.
set -euo pipefail

root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"

. "$root/herdr/locate.sh"

"$binary" fleet || {
  status=$?
  printf '\n  avocado-trope fleet exited with status %s\n\n' "$status" >&2
  read -r -p '  press enter to close ' _ || true
  exit "$status"
}
