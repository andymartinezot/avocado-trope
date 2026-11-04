# The fleet view, on made-up agents (assets/fleet/demo.json), so the recording
# never shows real sessions.
#   vhs scripts/demo/fleet.tape
Output docs/avocado-trope-fleet.gif

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
Type "./target/release/zoe fleet --demo assets/fleet/demo.json --interval 2"
Enter
Show

Sleep 8s
Type "a"
Sleep 3s
Escape
Sleep 1s
Type "s"
Sleep 800ms
Type "s"
Sleep 2s
Type "s"
Sleep 800ms
Type "/"
Sleep 300ms
Type "mobile"
Sleep 1500ms
Enter
Sleep 800ms
Escape
Sleep 800ms
Down
Sleep 500ms
Type "g"
Sleep 5s
Type "q"
Sleep 2s
