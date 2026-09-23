# Song-radio browsing evidence

Open [`index.html`](index.html) for the visual comparison. It has state, theme
and window-size selectors and Before/After controls. The states are the radio
page, the song menu open, loading, error with retry, and an empty station.

Baseline: upstream main `a65e85c`. Candidate: branch `codex/song-radio-browsing`.
All captures come from Linux (X11 under Xvfb) in demo mode. Each capture uses
fresh, isolated configuration and state. Windows are 1240x800 and 760x800.
Demo mode makes no Spotify requests.

The baseline has no radio page, so its loading, error and empty entries show
the playlist the action starts from.

Reproduce a capture on an isolated X11 display:

```sh
spotifast --demo --demo-data "$(mktemp -d)" \
  --demo-page radio:trk0 --demo-show light,radio-error --demo-size 760x800 \
  --demo-shot /tmp/radio.png --demo-shot-delay 2500
```

`--demo-show` accepts `radio-loading`, `radio-error` and `radio-empty`. Use
`--demo-page playlist:pl1` for the baseline page. For the menu captures, open
the context menu on the Shadows row, at (420, 578) in the normal window and
(420, 516) in the narrow window, before the shot is taken. These images check
layout only. They do not check station resolution while signed in, or
playback.
