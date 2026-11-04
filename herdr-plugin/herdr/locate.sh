# Sourced by the pane scripts: set $binary to the program to run.
#
# A checkout's own build wins, so a clone linked with `herdr plugin link` shows
# local changes after `cargo build --release`. Then the binary the plugin's
# build step installed, then whatever is on PATH.
binary=avocado-trope
for candidate in "$root/../target/release/avocado-trope" "$root/bin/avocado-trope"; do
  if [ -x "$candidate" ]; then
    binary="$candidate"
    break
  fi
done
