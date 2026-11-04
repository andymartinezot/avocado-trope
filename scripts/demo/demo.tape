# The session graph: replay, sidebar, session log, failures, detail, themes.
# Records the bundled demo session. From the repo root, after a release build:
#   vhs scripts/demo/demo.tape
Output docs/avocado-trope-demo.gif

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
Type "export XDG_CONFIG_HOME=$(mktemp -d); unset HERDR_PANE_ID AVOCADO_TROPE_THEME; clear"
Enter
Sleep 500ms
Type "./target/release/avocado-trope assets/claude/demo.jsonl --speed 16"
Enter
Show

Sleep 13s
Type "e"
Sleep 3s
Type "e"
Sleep 1s
Type "/"
Sleep 400ms
Type "npm"
Sleep 2s
Enter
Sleep 1s
Escape
Sleep 1s
Down
Sleep 3s
Escape
Sleep 1s
Type "t"
Sleep 2s
Type "t"
Sleep 2s
Type "t"
Sleep 2s
