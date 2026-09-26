#!/bin/sh
# Puts the build in your working tree where the installer would put it, so a change
# can be tried the way someone else will meet it: on PATH, as deplyd and dp, with
# completion wired up.
#
#   ./dev-install.sh              build, then stage it
#   ./dev-install.sh --release    the release profile, for a realistic binary
#   ./dev-install.sh --revert     take it back off
#
# This is not the installer and never will be. install.sh takes a published release
# and refuses anything it cannot verify; a local build is neither, so staging one is
# a separate job with a separate name. What it borrows is the tidying, so a dev
# install and a real one leave the same shape behind.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

profile=debug
revert=""
for argument in "$@"; do
    case "$argument" in
        --release) profile=release ;;
        --revert)  revert=1 ;;
        *) printf '\nUnknown option: %s\nUsage: dev-install.sh [--release] [--revert]\n\n' "$argument" >&2
           exit 1 ;;
    esac
done

# Everything install.sh defines, and nothing it does. The arguments are dropped
# first: a sourced script sees the caller's, and install.sh refuses ones it does
# not know.
set --
DEPLYD_SOURCE_ONLY=1
export DEPLYD_SOURCE_ONLY
# shellcheck source=install.sh
. "$root/install.sh"
unset DEPLYD_SOURCE_ONLY

if [ -n "$revert" ]; then
    uninstall
fi

# --- build ------------------------------------------------------------------

say ""
say "Building deplyd ($profile)"

if [ "$profile" = "release" ]; then
    (cd "$root" && cargo build --release)
else
    (cd "$root" && cargo build)
fi

built="$root/target/$profile/deplyd"
[ -f "$built" ] || fail "No binary at $built"

# --- stage it ---------------------------------------------------------------

say ""
mkdir -p "$INSTALL_DIR"

# Copied rather than linked, so rebuilding does not swap what is on PATH halfway
# through someone running it.
cp "$built" "$INSTALL_DIR/deplyd"
chmod +x "$INSTALL_DIR/deplyd"
say "  installed  $INSTALL_DIR/deplyd"

rm -f "$INSTALL_DIR/dp"
ln -s deplyd "$INSTALL_DIR/dp" 2>/dev/null || cp "$INSTALL_DIR/deplyd" "$INSTALL_DIR/dp"
say "  dp         short name for deplyd"

# --- PATH and completion ----------------------------------------------------

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) needs_path=1 ;;
esac

if [ -n "$rc" ]; then
    mkdir -p "$(dirname "$rc")"
    [ -f "$rc" ] || : > "$rc"
    strip_completions
    "$INSTALL_DIR/deplyd" completions "$shell" >> "$rc"
    say "  completion added to $rc"

    if [ -n "${needs_path:-}" ]; then
        if [ "$shell" = "fish" ]; then
            printf '%s\n' "fish_add_path $INSTALL_DIR" >> "$rc"
        else
            printf '%s\n' "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$rc"
        fi
        say "  path       added to $rc"
    fi
fi

say ""
say "Staged $("$INSTALL_DIR/deplyd" --version)."
say "  dp status                  try it"
say "  ./install.sh --uninstall   test the uninstaller against it"
say "  ./dev-install.sh --revert  take it back off"
say ""
