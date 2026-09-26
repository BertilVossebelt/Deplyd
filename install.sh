#!/bin/sh
# Installs deplyd on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh
#
# Downloads the release, checks it, puts deplyd and dp on your PATH, installs the
# GitHub CLI if it is missing, signs you in if you are not, and turns on completion.
# Only installing the GitHub CLI on Linux needs root, and it asks first.
#
# To undo all of that, pass --uninstall through sh:
#
#   curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh -s -- --uninstall
#
#   --uninstall          remove what the installer put there, and nothing else
#   --purge              with --uninstall, also remove settings and cache
#
#   DEPLYD_INSTALL_DIR   where to put it (default ~/.local/bin)
#   DEPLYD_VERSION       a tag to install (default the latest release)
#   DEPLYD_YES           answer yes to every question, for unattended installs
#   DEPLYD_REMOVE_GH     remove the GitHub CLI too, for an unattended uninstall

set -eu

REPO="BertilVossebelt/Deplyd"
INSTALL_DIR="${DEPLYD_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf '\n%s\n\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required and was not found."
}

# Piped to sh, so stdin is the script. Questions go to the terminal or are skipped.
confirm() {
    [ -n "${DEPLYD_YES:-}" ] && return 0
    [ -r /dev/tty ] || return 1
    printf '%s [Y/n] ' "$1" > /dev/tty
    read -r answer < /dev/tty || return 1
    case "$answer" in '' | y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

# For a question whose yes takes something away. DEPLYD_YES does not reach these:
# saying yes to an install is not saying yes to removing a tool other things use.
confirm_no() {
    # /dev/tty can exist and still not open. Two things to dodge: the shell reports
    # a redirection it could not make, so the group swallows that, and a redirection
    # error on a special builtin - ':' is one - ends a non-interactive shell rather
    # than failing. printf is not special, so this answers no instead of exiting.
    { printf '' > /dev/tty; } 2>/dev/null || return 1
    printf '%s [y/N] ' "$1" > /dev/tty
    read -r answer < /dev/tty || return 1
    case "$answer" in y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

# --- what this installer writes to a shell rc -------------------------------

START="# >>> deplyd completions >>>"
END="# <<< deplyd completions <<<"

case "${SHELL:-}" in
    */zsh)  rc="$HOME/.zshrc"; shell=zsh ;;
    */fish) rc="$HOME/.config/fish/config.fish"; shell=fish ;;
    */bash) rc="$HOME/.bashrc"; shell=bash ;;
    *)      rc=""; shell="" ;;
esac

# Installing and uninstalling both start by taking out what an earlier run left, so
# reinstalling does not stack up and uninstalling leaves nothing of ours behind.
strip_completions() {
    { [ -n "$rc" ] && [ -f "$rc" ]; } || return 0
    if grep -qF "$START" "$rc" 2>/dev/null; then
        sed -i.deplyd-bak "/^$START\$/,/^$END\$/d" "$rc" && rm -f "$rc.deplyd-bak"
    fi
    # A copy appended before the script carried markers ran to the end of the file,
    # appending being the only way it got there. Cut from where it starts.
    first=$(grep -nE '^(command -v dp >/dev/null|_deplyd\(\)|_dp\(\)|complete -c deplyd)' "$rc" 2>/dev/null |
        head -1 | cut -d: -f1)
    if [ -n "$first" ]; then
        head -n $((first - 1)) "$rc" > "$rc.deplyd-new" && mv "$rc.deplyd-new" "$rc"
    fi
}

# Only the line this installer appends, matched whole, so a PATH line written by
# hand is left where it is.
strip_path_line() {
    { [ -n "$rc" ] && [ -f "$rc" ]; } || return 0
    { grep -vxF "export PATH=\"$INSTALL_DIR:\$PATH\"" "$rc" 2>/dev/null || :; } |
        { grep -vxF "fish_add_path $INSTALL_DIR" || :; } > "$rc.deplyd-new"
    mv "$rc.deplyd-new" "$rc"
}

# --- uninstalling -----------------------------------------------------------

# dp is ours only if this installer made it: a link to the binary beside it, or a
# copy of it. Someone else's dp keeps the name on the way out as it did going in.
dp_is_ours() {
    [ -e "$INSTALL_DIR/dp" ] || return 1
    if [ -L "$INSTALL_DIR/dp" ]; then
        case "$(readlink "$INSTALL_DIR/dp")" in
            deplyd | "$INSTALL_DIR/deplyd") return 0 ;;
            *) return 1 ;;
        esac
    fi
    [ -f "$INSTALL_DIR/deplyd" ] && cmp -s "$INSTALL_DIR/dp" "$INSTALL_DIR/deplyd"
}

# Asked, never assumed: gh may well have been here first, and other things use it.
remove_gh() {
    command -v gh >/dev/null 2>&1 || return 0

    if [ -z "${DEPLYD_REMOVE_GH:-}" ]; then
        confirm_no "Remove the GitHub CLI as well? Other things may be using it." || {
            say "  gh         kept"
            return 0
        }
    fi

    # Whatever happens here, the uninstall carries on: gh is not ours, and a package
    # manager that says no about its own tool is not a reason to stop half way.
    if [ "$(uname -s)" = "Darwin" ]; then
        command -v brew >/dev/null 2>&1 && brew uninstall gh
    elif command -v apt-get >/dev/null 2>&1; then
        sudo apt-get remove -y gh
    elif command -v dnf >/dev/null 2>&1; then
        sudo dnf remove -y gh
    elif command -v pacman >/dev/null 2>&1; then
        sudo pacman -R --noconfirm github-cli
    elif command -v zypper >/dev/null 2>&1; then
        sudo zypper remove -y gh
    else
        false
    fi || {
        say "  gh         still here - remove it the way it was installed"
        return 0
    }

    # Removing the tool is not ours to read as revoking access: gh keeps its sign-in
    # in its own config, and it stays there.
    say "  gh         removed - its sign-in is still in ~/.config/gh"
}

uninstall() {
    say ""
    say "Removing deplyd"
    say ""

    if dp_is_ours; then
        rm -f "$INSTALL_DIR/dp"
        say "  dp         removed"
    elif [ -e "$INSTALL_DIR/dp" ]; then
        say "  dp         left alone, it is not the one this installer made"
    fi

    if [ -f "$INSTALL_DIR/deplyd" ]; then
        rm -f "$INSTALL_DIR/deplyd"
        say "  deplyd     removed from $INSTALL_DIR"
    else
        say "  deplyd     was not in $INSTALL_DIR"
    fi

    # The directory itself stays. Here it defaults to ~/.local/bin, shared with
    # everything else that installs there and never ours to have made.

    if [ -n "$rc" ] && [ -f "$rc" ]; then
        strip_completions
        strip_path_line
        say "  $rc tidied"
    fi

    config="${XDG_CONFIG_HOME:-$HOME/.config}/deplyd"
    if [ -n "$PURGE" ]; then
        # Guarded on the name, so a surprising XDG_CONFIG_HOME cannot point this
        # anywhere but at a directory called deplyd.
        case "$config" in
            */deplyd)
                if [ -d "$config" ]; then
                    rm -rf "$config"
                    say "  settings   removed from $config"
                fi
                ;;
        esac
    elif [ -d "$config" ]; then
        say "  settings   kept in $config, --purge removes them"
    fi

    # A .deplyd.json belongs to the repository it sits in, written where someone
    # asked for it rather than put there by this script.
    say "  repos      any .deplyd.json left where it is"

    remove_gh

    say ""
    say "Done. Open a new terminal."
    say ""
    exit 0
}

UNINSTALL=""
PURGE=""
for argument in "$@"; do
    case "$argument" in
        --uninstall) UNINSTALL=1 ;;
        --purge)     UNINSTALL=1; PURGE=1 ;;
        *) fail "Unknown option: $argument
Usage: install.sh [--uninstall] [--purge]" ;;
    esac
done

[ -n "$UNINSTALL" ] && uninstall

# Sourced by dev-install.sh, which wants the functions above and none of the work
# below. Stopping here is all it does: there is no path that installs anything the
# checks further down have not been through.
[ -n "${DEPLYD_SOURCE_ONLY:-}" ] && return 0

need curl
need tar

# --- what are we running on -------------------------------------------------

case "$(uname -s)" in
    Darwin) os=apple-darwin ;;
    Linux)  os=unknown-linux-gnu ;;
    MINGW* | MSYS* | CYGWIN*)
        fail "On Windows, use install.ps1:
  irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex" ;;
    *) fail "deplyd has no build for $(uname -s). From source: cargo install --path crates/deplyd" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64)  arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) fail "deplyd has no build for $(uname -m)." ;;
esac

TARGET="$arch-$os"

# --- which release ----------------------------------------------------------

if [ -n "${DEPLYD_VERSION:-}" ]; then
    TAG="$DEPLYD_VERSION"
else
    TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
        sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
    [ -n "$TAG" ] || fail "Could not work out the latest release. Set DEPLYD_VERSION to a tag."
fi

ARCHIVE="deplyd-$TAG-$TARGET.tar.gz"
BUNDLE="attestation.json"
BASE="https://github.com/$REPO/releases/download/$TAG"

say ""
say "deplyd $TAG for $TARGET"

# --- the GitHub CLI ---------------------------------------------------------

# Before the download rather than after it: gh is what checks the download, and a
# binary whose provenance cannot be checked is not one to install. Only the tool is
# wanted here. The check runs against a bundle published with the release, so it needs
# no account and no token, and signing in can wait until deplyd is on the disk.

say ""

install_gh() {
    if [ "$(uname -s)" = "Darwin" ]; then
        command -v brew >/dev/null 2>&1 || return 1
        brew install gh
        return $?
    fi
    # Root, so it asks before it does this and takes no for an answer.
    if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get update && sudo apt-get install -y gh
    elif command -v dnf >/dev/null 2>&1; then
        sudo dnf install -y gh
    elif command -v pacman >/dev/null 2>&1; then
        sudo pacman -S --noconfirm github-cli
    elif command -v zypper >/dev/null 2>&1; then
        sudo zypper install -y gh
    else
        return 1
    fi
}

if ! command -v gh >/dev/null 2>&1; then
    if confirm "deplyd reads GitHub through the GitHub CLI, which is not installed. Install it?"; then
        install_gh || true
    fi
fi

command -v gh >/dev/null 2>&1 || fail "The GitHub CLI is needed, to check this download and to run deplyd.
Install it from https://cli.github.com, then run this again."

# gh learned to verify attestations in 2.49. An older one cannot check, which is not
# the same answer as a check that failed, so say which it is.
gh attestation verify --help >/dev/null 2>&1 ||
    fail "This gh cannot check provenance - that arrived in 2.49. Update it, then run this again."

# --- download ---------------------------------------------------------------

WORK=$(mktemp -d)
# shellcheck disable=SC2064
trap "rm -rf '$WORK'" EXIT INT TERM

curl -fsSL "$BASE/$ARCHIVE" -o "$WORK/$ARCHIVE" ||
    fail "No build for $TARGET in $TAG. See https://github.com/$REPO/releases"

# --- check it is what was published ----------------------------------------

# Every release publishes SHA256SUMS, so a missing file or entry means this download
# cannot be shown to be the published one. Refuse rather than install it anyway.
curl -fsSL "$BASE/SHA256SUMS" -o "$WORK/SHA256SUMS" ||
    fail "Could not fetch SHA256SUMS for $TAG, so the download cannot be checked. Not installing."

expected=$(grep " $ARCHIVE\$" "$WORK/SHA256SUMS" | awk '{print $1}')
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $ARCHIVE. Not installing."

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$WORK/$ARCHIVE" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$WORK/$ARCHIVE" | awk '{print $1}')
else
    fail "Neither sha256sum nor shasum is here, so the download cannot be checked. Not installing."
fi

[ "$expected" = "$actual" ] || fail "Checksum mismatch. Not installing."
say "  checksum   ok"

# Signed through Sigstore and recorded in a public log, so this says the binary came
# from that repository's release workflow and not from somewhere else. The bundle is
# published with the release, which is what makes this check cost nothing to run: no
# account, no token, nothing to set up first.
if curl -fsSL "$BASE/$BUNDLE" -o "$WORK/$BUNDLE" 2>/dev/null; then
    gh attestation verify "$WORK/$ARCHIVE" --repo "$REPO" --bundle "$WORK/$BUNDLE" >/dev/null 2>&1 ||
        fail "Provenance check failed: $ARCHIVE is not what $REPO's release workflow built. Not installing."
elif gh auth status >/dev/null 2>&1; then
    # The early releases published no bundle. Ask GitHub for the attestation instead,
    # which works but wants the sign-in those releases could assume.
    gh attestation verify "$WORK/$ARCHIVE" --repo "$REPO" >/dev/null 2>&1 ||
        fail "Provenance check failed: $ARCHIVE is not what $REPO's release workflow built. Not installing."
else
    fail "$TAG published no attestation bundle, so checking it means asking GitHub.
Sign in with: gh auth login, or install the latest release, which carries its own."
fi
say "  provenance ok"

# --- install ----------------------------------------------------------------

tar -xzf "$WORK/$ARCHIVE" -C "$WORK"
[ -f "$WORK/deplyd" ] || fail "The archive did not contain a deplyd binary."

mkdir -p "$INSTALL_DIR"
mv "$WORK/deplyd" "$INSTALL_DIR/deplyd"
chmod +x "$INSTALL_DIR/deplyd"

say "  installed  $INSTALL_DIR/deplyd"

# dp is the short name, a symlink so it costs no disk. Someone else's dp keeps the
# name: a tool that is already there is not ours to take.
existing=$(command -v dp 2>/dev/null || true)
if [ -n "$existing" ] && [ "$existing" != "$INSTALL_DIR/dp" ]; then
    say "  dp         taken by $existing, skipped"
else
    rm -f "$INSTALL_DIR/dp"
    ln -s deplyd "$INSTALL_DIR/dp" 2>/dev/null || cp "$INSTALL_DIR/deplyd" "$INSTALL_DIR/dp"
    say "  dp         short name for deplyd"
fi

# --- PATH -------------------------------------------------------------------

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) PATH="$INSTALL_DIR:$PATH"; export PATH; NEEDS_PATH=1 ;;
esac

# --- signing in -------------------------------------------------------------

say ""

if gh auth status >/dev/null 2>&1; then
    say "  github     signed in"
elif confirm "You are not signed in to GitHub. Sign in now?"; then
    # Interactive on purpose: a device flow against access you already have.
    gh auth login < /dev/tty || true
else
    say "  github     not signed in - run: gh auth login"
fi

# --- completion -------------------------------------------------------------

if [ -n "$rc" ]; then
    mkdir -p "$(dirname "$rc")"
    [ -f "$rc" ] || : > "$rc"
    # Drop a previous block before writing this one, so reinstalling does not stack up.
    strip_completions
    # The markers come from the binary now, so they arrive with the script.
    "$INSTALL_DIR/deplyd" completions "$shell" >> "$rc"
    say "  completion added to $rc"
else
    say "  completion skipped - unknown shell. Run: deplyd completions bash >> ~/.bashrc"
fi

# --- done -------------------------------------------------------------------

say ""
if [ -n "${NEEDS_PATH:-}" ] && [ -n "$rc" ]; then
    if [ "$shell" = "fish" ]; then
        printf '%s\n' "fish_add_path $INSTALL_DIR" >> "$rc"
    else
        printf '%s\n' "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$rc"
    fi
    say "  path       added to $rc"
    say ""
fi

say "Done. Open a new terminal, then from inside any repo:"
say "  dp status"
say ""
