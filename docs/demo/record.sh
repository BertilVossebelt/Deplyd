#!/bin/sh
# Records docs/demo.gif from scratch.
#
# Nothing here reaches GitHub. build-demo.sh makes a throwaway repository and a
# GitHub-made-of-files; DEPLYD_STUB_DIR points deplyd at the latter. Every line
# of terminal output in the GIF is what the binary actually printed - only the
# timing is invented, and the notification is drawn (see draw-notify.py).
#
# Needs: agg (cargo install --git https://github.com/asciinema/agg) and a python
# with Pillow.
set -e

# Windows ships a python3 shim that exists but does not run, and the interpreter
# that has Pillow is not always first on PATH.
PY=""
for cand in python3 python py; do
  if "$cand" -c "import PIL" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "need a python with Pillow: pip install Pillow"; exit 1; }

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
# Somewhere short: deplyd prints absolute paths, and a temp path would fill the
# frame with scratch directories.
if [ -d /c ] && [ -w /c ]; then work="/c/deplyd-demo"; else work="${TMPDIR:-/tmp}/deplyd-demo"; fi

# Whichever build is newest: an old release binary lying around would otherwise
# be preferred over the debug build you just made.
bin=""
for cand in "$root/target/release/deplyd" "$root/target/release/deplyd.exe" \
            "$root/target/debug/deplyd" "$root/target/debug/deplyd.exe"; do
  [ -f "$cand" ] || continue
  if [ -z "$bin" ] || [ "$cand" -nt "$bin" ]; then bin="$cand"; fi
done
[ -n "$bin" ] || { echo "build deplyd first: cargo build --release"; exit 1; }
echo "deplyd: $bin"

sh "$here/build-demo.sh" "$work" >/dev/null

# The sandbox environment lives in this subshell only. Exporting APPDATA in the
# parent would hide python's user site-packages, and with it Pillow.
(
  cd "$work/repo"
  export HOME="$work/home" USERPROFILE="$work/home" APPDATA="$work/home"
  export XDG_CONFIG_HOME="$work/home" GIT_CONFIG_GLOBAL="$work/home/gitconfig"
  export GIT_CONFIG_NOSYSTEM=1 GIT_ALLOW_PROTOCOL=file GIT_TERMINAL_PROMPT=0
  export DEPLYD_STUB_DIR="$work/stub" COLUMNS=100 CLICOLOR_FORCE=1

  # deplyd exits 2 when a change is not deployed, which is the point of scene 2.
  "$bin" status        > "$work/sc1.ansi" 2>&1 || true
  "$bin" status pr 412 > "$work/sc2.ansi" 2>&1 || true

  # Scene 3: register the notifier, so the toast in scene 4 has a visible cause.
  "$bin" hooks add ./hooks/notify.ps1 > "$work/sc3.ansi" 2>&1 || true

  # Scene 4 for real: start watching, then let WEB ship underneath it by copying
  # the after-state over the stub. The watcher notices on its next look.
  # A real interval, waited out for real: the deploy is copied in during it and
  # the next look finds it. mkcast.py drops the wait from the GIF's timeline.
  "$bin" watch -E production -A Bertil --every 60s > "$work/sc4.ansi" 2>&1 &
  wpid=$!
  sleep 15
  cp "$work/after/"* "$work/stub/"
  echo "waiting out the 60s watch interval..."
  sleep 58
  kill $wpid 2>/dev/null || true
  wait $wpid 2>/dev/null || true
)

"$PY" "$here/draw-notify.py"
"$PY" "$here/mkcast.py" "$work"
agg --font-size 15 --line-height 1.35 "$work/demo.cast" "$work/base.gif"
"$PY" "$here/compose.py" "$work/base.gif" "$root/docs/demo.gif"
echo "wrote docs/demo.gif"
