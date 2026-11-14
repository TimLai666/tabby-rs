#!/usr/bin/env bash
# Linux desktop acceptance for the ported host features, under Xvfb + openbox.
# Each case runs a fresh portable copy with its own data dir; nothing touches the
# user's real config. Prints PASS/FAIL lines and a summary.
set -u
# Usage: TABBY_RS_BINARY=/path/to/tabby-rs [TABBY_RS_ACCEPTANCE_ROOT=/tmp/dir] [DISPLAY=:77] \
#        scripts/acceptance/x11-linux-parity.sh [all|devtools|gpu|hotkey]
# Needs Xvfb, openbox, dbus-daemon and xdotool. Cases: `-d` opens the Web Inspector,
# hacks.disableGPU reaches WebKitWebProcess but not a terminal shell, and a configured
# toggle-window hotkey hides and shows the focused window on X11.
ROOT=${TABBY_RS_ACCEPTANCE_ROOT:-/tmp/tabby-rs-acceptance}
APP=${TABBY_RS_BINARY:?set TABBY_RS_BINARY to the tabby-rs executable}
mkdir -p "$ROOT"
export DISPLAY=${DISPLAY:-:77}
ONLY=${1:-all}
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }
ko(){ echo "FAIL $1"; fail=$((fail+1)); }

Xvfb "$DISPLAY" -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 & XVFB=$!
sleep 1
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address 2>/dev/null)
DBUS_SESSION_BUS_PID=$(pgrep -n -f "dbus-daemon --session --fork")
openbox >/tmp/openbox.log 2>&1 & OB=$!
sleep 1

fresh_case(){ # name -> prints portable dir
  local d=$ROOT/case-$1; rm -rf "$d"; mkdir -p "$d/data"; cp "$APP" "$d/tabby-rs"; echo "$d"
}
app_windows(){ # pid -> visible window ids of that process
  xdotool search --onlyvisible --pid "$1" 2>/dev/null
}
wait_window(){ # pid timeout -> 0 once the process shows a window
  local pid=$1 n=${2:-30} i=0
  while [ $i -lt "$n" ]; do
    if [ -n "$(app_windows "$pid")" ]; then return 0; fi
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 1; i=$((i+1))
  done
  return 1
}
stop_app(){ kill "$1" 2>/dev/null; sleep 2; kill -9 "$1" 2>/dev/null; sleep 1; }

# Case 1: -d opens DevTools (WebKit inspector window) alongside the main window.
if [ "$ONLY" = all ] || [ "$ONLY" = devtools ]; then
D=$(fresh_case devtools)
"$D/tabby-rs" -d >"$D/app.log" 2>&1 & PID=$!
if wait_window $PID 60; then
  sleep 4
  all=$(xdotool search --onlyvisible --name "." 2>/dev/null | while read -r w; do xdotool getwindowname "$w"; done | sort -u)
  echo "  windows: $(echo "$all" | tr '\n' '|')"
  if echo "$all" | grep -qiE "inspector|devtools|developer"; then ok "devtools: -d opens a Web Inspector window"; else ko "devtools: no inspector window after -d"; fi
else ko "devtools: main window did not appear"; fi
stop_app $PID
fi

# Case 2: hacks.disableGPU exports the WebKit variables to the WebView processes
# but not to a terminal shell. /proc/<app>/environ is the exec-time snapshot, so the
# app's own entry is not checked; children spawned after startup are.
if [ "$ONLY" = all ] || [ "$ONLY" = gpu ]; then
D=$(fresh_case gpu)
printf 'version: 1\nhacks:\n  disableGPU: true\n' > "$D/data/config.yaml"
"$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
if wait_window $PID 60; then
  sleep 5
  web=$(pgrep -P $PID -f WebKitWebProcess | head -1)
  if [ -n "$web" ] && tr '\0' '\n' < /proc/$web/environ | grep -q '^WEBKIT_DISABLE_COMPOSITING_MODE=1$' && tr '\0' '\n' < /proc/$web/environ | grep -q '^WEBKIT_DISABLE_DMABUF_RENDERER=1$'; then ok "gpu: WebKitWebProcess carries both WebKit variables"; else ko "gpu: WebKitWebProcess lacks the WebKit variables"; [ -n "$web" ] && tr '\0' '\n' < /proc/$web/environ | grep -i "webkit_disable\|TABBY_RS" ; fi
  # Open a terminal tab (default new-tab hotkey) and inspect its shell.
  win=$(app_windows $PID | head -1); xdotool windowactivate --sync "$win" 2>/dev/null; sleep 1
  xdotool key ctrl+shift+t; sleep 5
  found_shell=0
  for s in $(pgrep -P $PID); do
    cmd=$(tr '\0' ' ' < /proc/$s/cmdline 2>/dev/null)
    case "$cmd" in *WebKit*) continue;; esac
    case "$cmd" in *bash*|*zsh*|*fish*|*sh*|*login*) found_shell=1
      if tr '\0' '\n' < /proc/$s/environ | grep -q 'WEBKIT_DISABLE\|TABBY_RS_GPU_ENV'; then ko "gpu: shell $s ($cmd) inherited the WebKit variables"; else ok "gpu: shell $s ($cmd) does not see the WebKit variables"; fi;;
    esac
  done
  [ $found_shell = 1 ] || { echo "  children: $(for s in $(pgrep -P $PID); do tr '\0' ' ' < /proc/$s/cmdline; echo -n ' ; '; done)"; ko "gpu: no shell child found after opening a tab"; }
else ko "gpu: main window did not appear"; fi
stop_app $PID
fi

# Case 3: a configured toggle-window global hotkey hides and shows the window on X11.
if [ "$ONLY" = all ] || [ "$ONLY" = hotkey ]; then
D=$(fresh_case hotkey)
printf 'version: 1\nhotkeys:\n  toggle-window:\n    - Ctrl-Alt-T\n' > "$D/data/config.yaml"
"$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
if wait_window $PID 60; then
  sleep 4
  xdotool key ctrl+alt+t; sleep 2
  if [ -n "$(app_windows $PID)" ]; then ko "hotkey: window still visible after Ctrl+Alt+T"; else ok "hotkey: Ctrl+Alt+T hides the window"; fi
  xdotool key ctrl+alt+t; sleep 2
  if [ -n "$(app_windows $PID)" ]; then ok "hotkey: Ctrl+Alt+T shows the window again"; else ko "hotkey: window not shown again"; fi
else ko "hotkey: main window did not appear"; fi
stop_app $PID
fi

kill $OB $XVFB 2>/dev/null; kill "$DBUS_SESSION_BUS_PID" 2>/dev/null
echo "SUMMARY pass=$pass fail=$fail"
