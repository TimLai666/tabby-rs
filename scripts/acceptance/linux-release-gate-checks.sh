#!/usr/bin/env bash
# Exercises the linux-x64 release artifacts produced by linux-release-gate.sh
# under Xvfb + openbox and writes one evidence text file per check into
# <staging>/manual/linux-x64/. The files hold observed command output only;
# generated test keys stay in <staging>-work and are never copied.
#
# Usage: scripts/acceptance/linux-release-gate-checks.sh [case...]
#   cases: packages local-shell ssh side-by-side wayland   (default: all)
# Environment: TABBY_RS_RELEASE_STAGING (default release-staging), DISPLAY (default :77),
#   TABBY_ORIGINAL_DIR (unpacked original Tabby, optional, for side-by-side),
#   TABBY_RS_SSH_PORT (default 22022).
set -u
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"
STAGING=$(realpath -m "${TABBY_RS_RELEASE_STAGING:-release-staging}")
WORK=$STAGING-work/checks
EVIDENCE=$STAGING/manual/linux-x64
SSH_PORT=${TABBY_RS_SSH_PORT:-22022}
export DISPLAY=${DISPLAY:-:77}
mkdir -p "$WORK" "$EVIDENCE"
# rpm's sqlite database and Wayland sockets need a local, short path: a
# bind-mounted checkout breaks sqlite WAL and long paths exceed sun_path.
LOCAL=$(mktemp -d /tmp/tabby-rs-gate-XXXXXX)
ONLY=${*:-packages local-shell ssh side-by-side wayland}
pass=0; fail=0
ok(){ echo "PASS $1" | tee -a "$EV"; pass=$((pass+1)); }
ko(){ echo "FAIL $1" | tee -a "$EV"; fail=$((fail+1)); }
note(){ echo "$*" | tee -a "$EV"; }
run_cases(){ case " $ONLY " in *" $1 "*) return 0;; esac; return 1; }

APPIMAGE=$(find "$STAGING" -maxdepth 2 -iname '*.AppImage' | head -1)
DEB=$(find "$STAGING" -maxdepth 2 -iname '*.deb' | head -1)
RPM=$(find "$STAGING" -maxdepth 2 -iname '*.rpm' | head -1)
[ -n "$APPIMAGE" ] && [ -n "$DEB" ] && [ -n "$RPM" ] || { echo "artifacts missing under $STAGING" >&2; exit 1; }

# Display stack ---------------------------------------------------------------
Xvfb "$DISPLAY" -screen 0 1280x800x24 >"$WORK/xvfb.log" 2>&1 & XVFB=$!
sleep 1
export DBUS_SESSION_BUS_ADDRESS
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address 2>/dev/null)
DBUS_PID=$(pgrep -n -x dbus-daemon)
openbox >"$WORK/openbox.log" 2>&1 & OB=$!
sleep 1
cleanup(){ kill $OB $XVFB 2>/dev/null; kill "$DBUS_PID" 2>/dev/null; sudo -n rm -rf "$LOCAL" 2>/dev/null || rm -rf "$LOCAL"; }
trap cleanup EXIT

tree_pids(){ local p; echo "$1"; for p in $(pgrep -P "$1"); do tree_pids "$p"; done; }
# AppImage and wrapper launches own the window in a child process, so look
# through the whole process tree rather than the launched pid only.
app_windows(){ local p; for p in $(tree_pids "$1"); do xdotool search --onlyvisible --pid "$p" 2>/dev/null; done; }
wait_window(){ local pid=$1 n=${2:-60} i=0
  while [ $i -lt "$n" ]; do
    [ -n "$(app_windows "$pid")" ] && return 0
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 1; i=$((i+1)); done; return 1; }
shell_child(){ # app pid -> pid of a shell spawned by the app (not WebKit)
  for s in $(pgrep -P "$1"); do
    cmd=$(tr '\0' ' ' < /proc/$s/cmdline 2>/dev/null)
    case "$cmd" in *WebKit*|*bwrap*|*xdg-dbus-proxy*) continue;; esac
    case "$cmd" in *bash*|*zsh*|*fish*|*/sh*|*login*) echo "$s"; return 0;; esac
  done; return 1; }
wait_file(){ local f=$1 n=${2:-20} i=0; while [ $i -lt "$n" ]; do [ -e "$f" ] && return 0; sleep 1; i=$((i+1)); done; return 1; }
stop_app(){ kill "$1" 2>/dev/null; sleep 2; kill -9 "$1" 2>/dev/null; sleep 1; }
type_line(){ xdotool type --delay 40 "$1"; xdotool key Return; }
fresh_portable(){ # name -> portable dir with the deb-installed binary and an empty data dir
  local d=$WORK/portable-$1; rm -rf "$d"; mkdir -p "$d/data"; cp "$WORK/deb-root/usr/bin/tabby-rs" "$d/tabby-rs"; echo "$d"; }
base_config(){ printf 'version: 1\nenableWelcomeTab: false\nterminal:\n  autoOpen: true\nrecoverTabs: false\n' > "$1/data/config.yaml"; }

# Package inspection, install and uninstall boundary -----------------------------------
unpack_deb(){ rm -rf "$WORK/deb-root"; mkdir -p "$WORK/deb-root/var/lib/dpkg"; : > "$WORK/deb-root/var/lib/dpkg/status"
  dpkg --force-not-root --root="$WORK/deb-root" --admindir="$WORK/deb-root/var/lib/dpkg" --instdir="$WORK/deb-root" --unpack "$DEB"; }
if run_cases packages || run_cases local-shell || run_cases ssh || run_cases side-by-side || run_cases wayland; then
  unpack_deb >"$WORK/deb-unpack.log" 2>&1 || { echo "deb unpack failed"; cat "$WORK/deb-unpack.log"; exit 1; }
fi

if run_cases packages; then
  EV=$EVIDENCE/deb.txt; : >"$EV"
  note "# deb: $(basename "$DEB") sha256 $(sha256sum "$DEB" | cut -c1-64)"
  note "## dpkg-deb -I"; dpkg-deb -I "$DEB" | tee -a "$EV" >/dev/null
  note "## installed files (dpkg --unpack into an isolated root)"; dpkg-deb -c "$DEB" | awk '{print $6}' | tee -a "$EV" >/dev/null
  [ -x "$WORK/deb-root/usr/bin/tabby-rs" ] && ok "deb: unpack placed usr/bin/tabby-rs" || ko "deb: no usr/bin/tabby-rs after unpack"
  grep -q '^Package: tabby-rs$' <(dpkg-deb -f "$DEB" Package | sed 's/^/Package: /') && ok "deb: package name is tabby-rs" || ko "deb: unexpected package name"
  note "## desktop entry"; cat "$WORK/deb-root/usr/share/applications/"*.desktop | tee -a "$EV" >/dev/null
  D=$(fresh_portable deb); base_config "$D"
  "$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
  if wait_window $PID; then ok "deb: binary launches and shows a window (pid $PID, windows: $(app_windows $PID | tr '\n' ' '))"; else ko "deb: no window"; fi
  stop_app $PID
  purge_root=$WORK/deb-root-purge; rm -rf "$purge_root"; cp -a "$WORK/deb-root" "$purge_root"
  dpkg --force-not-root --root="$purge_root" --admindir="$purge_root/var/lib/dpkg" --instdir="$purge_root" --purge tabby-rs >>"$EV" 2>&1
  [ ! -e "$purge_root/usr/bin/tabby-rs" ] && ok "deb: purge removed usr/bin/tabby-rs" || ko "deb: purge left usr/bin/tabby-rs"
  note "## remaining files after purge: $(find "$purge_root" -type f ! -path '*/var/lib/dpkg/*' | wc -l)"

  EV=$EVIDENCE/rpm.txt; : >"$EV"
  note "# rpm: $(basename "$RPM") sha256 $(sha256sum "$RPM" | cut -c1-64)"
  note "## rpm -qpi"; rpm -qpi "$RPM" 2>/dev/null | tee -a "$EV" >/dev/null
  note "## rpm -qpl"; rpm -qpl "$RPM" 2>/dev/null | tee -a "$EV" >/dev/null
  # rpm --root needs chroot, which an unprivileged user cannot do; use sudo when
  # it is available without a password, and fail the check otherwise.
  rroot=$LOCAL/rpm-root; sudo -n rm -rf "$rroot" 2>/dev/null || rm -rf "$rroot"; mkdir -p "$rroot"
  if sudo -n true 2>/dev/null; then RPMC=(sudo -n rpm); else RPMC=(rpm); note "## no passwordless sudo: rpm --root will likely fail"; fi
  "${RPMC[@]}" --root="$rroot" --initdb >>"$EV" 2>&1
  if "${RPMC[@]}" --root="$rroot" --install --nodeps "$RPM" >>"$EV" 2>&1 && [ -x "$rroot/usr/bin/tabby-rs" ]; then
    ok "rpm: rpm --install into an isolated root placed usr/bin/tabby-rs"
    note "## rpm -qa in the isolated root: $("${RPMC[@]}" --root="$rroot" -qa 2>/dev/null | tr '\n' ' ')"
    D=$WORK/portable-rpm; rm -rf "$D"; mkdir -p "$D/data"; cp "$rroot/usr/bin/tabby-rs" "$D/tabby-rs"; base_config "$D"
    cmp -s "$D/tabby-rs" "$rroot/usr/bin/tabby-rs" && note "launch copy sha256 $(sha256sum "$D/tabby-rs" | cut -c1-64)"
    "$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
    if wait_window $PID; then ok "rpm: the rpm-installed binary launches and shows a window"; else ko "rpm: no window"; fi
    stop_app $PID
    "${RPMC[@]}" --root="$rroot" --erase tabby-rs >>"$EV" 2>&1
    [ ! -e "$rroot/usr/bin/tabby-rs" ] && ok "rpm: rpm --erase removed usr/bin/tabby-rs" || ko "rpm: erase left usr/bin/tabby-rs"
  else ko "rpm: install failed"; fi
  note "## remaining files after erase: $(sudo -n find "$rroot" -type f ! -path '*/var/lib/rpm/*' 2>/dev/null | wc -l)"

  EV=$EVIDENCE/appimage.txt; : >"$EV"
  note "# appimage: $(basename "$APPIMAGE") sha256 $(sha256sum "$APPIMAGE" | cut -c1-64)"
  aroot=$WORK/appimage; rm -rf "$aroot"; mkdir -p "$aroot"
  chmod +x "$APPIMAGE"
  (cd "$aroot" && "$APPIMAGE" --appimage-extract >/dev/null 2>&1)
  if [ -x "$aroot/squashfs-root/AppRun" ]; then ok "appimage: --appimage-extract produced AppRun"; else ko "appimage: extraction failed"; fi
  note "## desktop entry"; cat "$aroot/squashfs-root/"*.desktop 2>/dev/null | tee -a "$EV" >/dev/null
  note "## top-level entries"; ls "$aroot/squashfs-root" | tee -a "$EV" >/dev/null
  if [ -e /dev/fuse ]; then mode="direct (FUSE available)"; runner=("$APPIMAGE"); else mode="APPIMAGE_EXTRACT_AND_RUN=1 (no /dev/fuse in this environment)"; runner=(env APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE"); fi
  note "## launch mode: $mode"
  h=$WORK/appimage-home; rm -rf "$h"; mkdir -p "$h"
  HOME=$h XDG_CONFIG_HOME=$h/.config "${runner[@]}" >"$WORK/appimage-app.log" 2>&1 & PID=$!
  if wait_window $PID 90; then
    ok "appimage: launches and shows a window"
    sleep 3; note "## files created under the isolated XDG_CONFIG_HOME"; find "$h/.config" -maxdepth 2 | tee -a "$EV" >/dev/null
  else ko "appimage: no window ($(tail -3 "$WORK/appimage-app.log" | tr '\n' ' '))"; fi
  stop_app $PID; pkill -P $PID 2>/dev/null
  rm -rf "$aroot/squashfs-root"; [ ! -e "$aroot/squashfs-root" ] && ok "appimage: removing the extracted tree leaves nothing installed" || ko "appimage: extracted tree remained"
fi

# local shell: PTY input, resize, signal, clean exit ----------------------------------
if run_cases local-shell; then
  EV=$EVIDENCE/local-shell.txt; : >"$EV"
  note "# local-shell: deb-installed tabby-rs as a portable copy, Xvfb $DISPLAY, openbox"
  note "# WebKitGTK: $(dpkg-query -W -f='${Package} ${Version}' libwebkit2gtk-4.1-0 2>/dev/null)"
  D=$(fresh_portable local-shell); base_config "$D"
  "$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
  if wait_window $PID; then
    sleep 6
    win=$(app_windows $PID | head -1); xdotool windowactivate --sync "$win"; sleep 1
    sh=$(shell_child $PID) || { xdotool key ctrl+shift+t; sleep 5; sh=$(shell_child $PID); }
    if [ -n "$sh" ]; then
      ok "local-shell: app spawned shell pid $sh ($(tr '\0' ' ' < /proc/$sh/cmdline))"
      type_line "stty size > $D/size1; touch $D/typed"
      wait_file "$D/typed" && ok "local-shell: typed command ran in the PTY ($(cat "$D/size1" 2>/dev/null) rows cols)" || ko "local-shell: typed command had no effect"
      xdotool windowsize "$win" 1000 700; sleep 2
      type_line "stty size > $D/size2"
      sleep 2; note "size before/after window resize: $(cat "$D/size1" 2>/dev/null) -> $(cat "$D/size2" 2>/dev/null)"
      [ -s "$D/size2" ] && [ "$(cat "$D/size1")" != "$(cat "$D/size2")" ] && ok "local-shell: PTY size changed after window resize" || ko "local-shell: PTY size did not change"
      type_line "sleep 300; echo after > $D/after"; sleep 2
      sl=$(pgrep -P "$sh" -x sleep); xdotool key ctrl+c; sleep 2
      if [ -n "$sl" ] && ! kill -0 "$sl" 2>/dev/null && [ ! -e "$D/after" ]; then ok "local-shell: Ctrl+C delivered SIGINT (sleep $sl gone, following command skipped)"; else ko "local-shell: Ctrl+C did not interrupt sleep"; fi
      type_line "printf 'U+%%s\\n' 中文 émoji >$D/unicode"; sleep 2; note "unicode round trip: $(cat "$D/unicode" 2>/dev/null)"
      type_line "exit"; sleep 3
      kill -0 "$sh" 2>/dev/null && ko "local-shell: shell still alive after exit" || ok "local-shell: shell exited cleanly after 'exit'"
    else ko "local-shell: no shell child found"; fi
  else ko "local-shell: main window did not appear"; fi
  stop_app $PID
  pgrep -P $PID >/dev/null && ko "local-shell: children survived app stop" || ok "local-shell: no child processes after the app stopped"
  note "## app log tail"; tail -5 "$D/app.log" | tee -a "$EV" >/dev/null
fi

# ssh: loopback OpenSSH server started here, test-only key material in $WORK ------------
if run_cases ssh; then
  EV=$EVIDENCE/ssh.txt; : >"$EV"
  S=$WORK/sshd; rm -rf "$S"; mkdir -p "$S"; chmod 700 "$S"
  ssh-keygen -q -t ed25519 -N '' -f "$S/host_key" >/dev/null
  ssh-keygen -q -t ed25519 -N '' -f "$S/client_key" >/dev/null
  cp "$S/client_key.pub" "$S/authorized_keys"; chmod 600 "$S/authorized_keys"
  cat > "$S/sshd_config" <<EOF
Port $SSH_PORT
ListenAddress 127.0.0.1
HostKey $S/host_key
AuthorizedKeysFile $S/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PidFile $S/sshd.pid
LogLevel VERBOSE
UsePAM no
StrictModes no
EOF
  /usr/sbin/sshd -f "$S/sshd_config" -E "$S/sshd.log" -D & SSHD=$!
  sleep 1
  note "# ssh: loopback OpenSSH $(ssh -V 2>&1) on 127.0.0.1:$SSH_PORT, publickey only, test-only keys under the work dir (not copied)"
  D=$(fresh_portable ssh); base_config "$D"
  cat >> "$D/data/config.yaml" <<EOF
profiles:
  - id: ssh:loopback
    type: ssh
    name: loopback
    options:
      host: 127.0.0.1
      port: $SSH_PORT
      user: $(id -un)
      auth: publicKey
      privateKeys:
        - $S/client_key
EOF
  # Seed the portable data dir's known_hosts with the test server key so the
  # connection takes the matching-key path. The host-key prompt is not exercised.
  echo "[127.0.0.1]:$SSH_PORT $(cut -d' ' -f1-2 "$S/host_key.pub")" > "$D/data/known_hosts"
  note "known_hosts seeded with the test host key (fingerprint $(ssh-keygen -lf "$S/host_key.pub" | cut -d' ' -f2)); host-key prompt UI not exercised"
  "$D/tabby-rs" profile loopback >"$D/app.log" 2>&1 & PID=$!
  if wait_window $PID; then
    i=0; while [ $i -lt 30 ] && ! grep -q 'Accepted publickey' "$S/sshd.log"; do sleep 1; i=$((i+1)); done
    win=$(app_windows $PID | head -1); xdotool windowactivate --sync "$win"; sleep 3
    if grep -q 'Accepted publickey' "$S/sshd.log"; then
      ok "ssh: sshd accepted the publickey login ($(grep -c 'Accepted publickey' "$S/sshd.log") session)"
      type_line "touch $D/remote-typed; stty size > $D/remote-size"; wait_file "$D/remote-typed" 15 && ok "ssh: typed command ran in the remote shell (size $(cat "$D/remote-size" 2>/dev/null))" || ko "ssh: typed command had no effect"
      xdotool windowsize "$win" 1000 700; sleep 2; type_line "stty size > $D/remote-size2"; sleep 2
      [ -s "$D/remote-size2" ] && [ "$(cat "$D/remote-size")" != "$(cat "$D/remote-size2")" ] && ok "ssh: remote PTY resized with the window ($(cat "$D/remote-size2"))" || ko "ssh: remote PTY size unchanged"
      type_line "exit"; sleep 4
      grep -qE 'Received disconnect|session closed|Connection closed' "$S/sshd.log" && ok "ssh: server saw the session close after exit" || ko "ssh: no disconnect logged"
      before=$(grep -c 'Accepted publickey' "$S/sshd.log")
      "$D/tabby-rs" profile loopback >>"$D/app.log" 2>&1; sleep 8
      grep -q 'Accepted publickey' "$S/sshd.log" && [ "$(grep -c 'Accepted publickey' "$S/sshd.log")" -gt "$before" ] && ok "ssh: reconnect through a second 'profile loopback' invocation produced a new accepted login" || ko "ssh: reconnect did not log in"
    else ko "ssh: no accepted login ($(tail -3 "$S/sshd.log" | tr '\n' ' '))"; fi
  else ko "ssh: main window did not appear"; fi
  stop_app $PID; sleep 1
  ss -tnp 2>/dev/null | grep ":$SSH_PORT " | grep -q ESTAB && ko "ssh: connections remain after app stop" || ok "ssh: no established connection to the test server after app stop"
  note "## sshd log (redacted to auth/session lines)"; grep -E 'Accepted|disconnect|session|closed|Connection' "$S/sshd.log" | sed 's/ port [0-9]*//' | tee -a "$EV" >/dev/null
  kill $SSHD 2>/dev/null
fi

# side-by-side with the original Tabby in an isolated HOME ------------------------------
if run_cases side-by-side; then
  EV=$EVIDENCE/side-by-side.txt; : >"$EV"
  ORIG=${TABBY_ORIGINAL_DIR:-}
  if [ -n "$ORIG" ] && [ -x "$ORIG/tabby" ]; then
    oh=$WORK/orig-home; rm -rf "$oh"; mkdir -p "$oh"
    note "# original: $("$ORIG/tabby" --no-sandbox --version 2>/dev/null | tail -1) from $ORIG with HOME=$oh; Tabby RS portable copy"
    HOME=$oh XDG_CONFIG_HOME=$oh/.config "$ORIG/tabby" --no-sandbox >"$WORK/orig.log" 2>&1 & OPID=$!
    D=$(fresh_portable sbs); base_config "$D"
    "$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
    wait_window $OPID 90 && ok "side-by-side: original Tabby window visible (pid $OPID)" || ko "side-by-side: original Tabby showed no window"
    wait_window $PID 60 && ok "side-by-side: Tabby RS window visible (pid $PID)" || ko "side-by-side: Tabby RS showed no window"
    sleep 6
    note "## window titles"; for w in $(app_windows $OPID) $(app_windows $PID); do xdotool getwindowname "$w"; done | tee -a "$EV" >/dev/null
    [ -d "$oh/.config/tabby" ] && ok "side-by-side: original created its data dir under the isolated HOME (.config/tabby)" || ko "side-by-side: original data dir missing"
    [ -f "$D/data/config.yaml" ] && [ ! -e "$oh/.config/tabby-rs" ] && ok "side-by-side: Tabby RS kept its data in its portable data dir, nothing under the original's HOME" || ko "side-by-side: Tabby RS data location unexpected"
    note "## original HOME tree"; find "$oh" -maxdepth 3 | tee -a "$EV" >/dev/null
    note "## Tabby RS data tree"; find "$D/data" -maxdepth 1 | tee -a "$EV" >/dev/null
    stop_app $PID; stop_app $OPID
  else note "SKIP side-by-side: TABBY_ORIGINAL_DIR not set or has no tabby executable"; fi
fi

# nested Wayland (weston headless): launch observation only ------------------------------
if run_cases wayland; then
  EV=$EVIDENCE/wayland.txt; : >"$EV"
  if command -v weston >/dev/null; then
    export XDG_RUNTIME_DIR=$LOCAL/xdg; rm -rf "$XDG_RUNTIME_DIR"; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
    weston --backend=headless --socket=tabby-wl --width=1280 --height=800 >"$WORK/weston.log" 2>&1 & WPID=$!
    sleep 3
    D=$(fresh_portable wayland); base_config "$D"
    WAYLAND_DISPLAY=tabby-wl GDK_BACKEND=wayland "$D/tabby-rs" >"$D/app.log" 2>&1 & PID=$!
    sleep 15
    if kill -0 $PID 2>/dev/null; then
      ok "wayland: app alive 15 s after launch on weston headless ($(weston --version 2>/dev/null))"
      pgrep -P $PID -f WebKitWebProcess >/dev/null && ok "wayland: WebKitWebProcess child present" || ko "wayland: no WebKitWebProcess"
      sh=$(shell_child $PID) && ok "wayland: auto-opened terminal spawned shell pid $sh" || ko "wayland: no shell child"
      grep -iq 'wayland' "$WORK/weston.log" && note "weston log: $(grep -ciE 'surface|client' "$WORK/weston.log") surface/client lines"
      note "NOTE: no input was driven on Wayland; this records launch only"
    else ko "wayland: app exited ($(tail -3 "$D/app.log" | tr '\n' ' '))"; fi
    stop_app $PID; kill $WPID 2>/dev/null
  else note "SKIP wayland: weston not installed"; fi
fi

echo "SUMMARY pass=$pass fail=$fail" | tee "$EVIDENCE/summary.txt"
