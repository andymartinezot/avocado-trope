# Two sessions side by side on one clock, from the bundled recordings.
#   vhs scripts/demo/compare.tape
Output docs/avocado-trope-compare.gif

Set Shell "bash"
Set FontSize 14
Set Width 1440
Set Height 800
Set Padding 0
Set Framerate 15
Set TypingSpeed 80ms

# Hide the launch: open on the app, with a throwaway config so `t` leaves the
# real remembered theme alone.
Hide
Type "export XDG_CONFIG_HOME=$(mktemp -d); unset HERDR_PANE_ID ZOE_THEME; clear"
Enter
Sleep 500ms
Type "./target/release/zoe compare assets/claude/demo.jsonl assets/codex/cli-0.149.1/2026/08/26/rollout-2026-08-26T18-34-34-01a03eb5-7309-71a1-b9ea-3d76ee9f4968.jsonl --speed 24"
Enter
Show

Sleep 14s
Type "e"
Sleep 3s
Type "e"
Sleep 1s
Type "o"
Sleep 2s
