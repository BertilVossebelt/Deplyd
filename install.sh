#!/bin/sh
# Installs deplyd on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh
#
# Downloads the release, checks it, puts deplyd and dp on your PATH, installs the
# GitHub CLI if it is missing, signs you in if you are not, and turns on completion.
# Only installing the GitHub CLI on Linux needs root, and it asks first.
#
#   DEPLYD_INSTALL_DIR   where to put it (default ~/.local/bin)
#   DEPLYD_VERSION       a tag to install (default the latest release)
#   DEPLYD_YES           answer yes to every question, for unattended installs

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
BASE="https://github.com/$REPO/releases/download/$TAG"

say ""
say "deplyd $TAG for $TARGET"

# --- download ---------------------------------------------------------------

WORK=$(mktemp -d)
# shellcheck disable=SC2064
trap "rm -rf '$WORK'" EXIT INT TERM

curl -fsSL "$BASE/$ARCHIVE" -o "$WORK/$ARCHIVE" ||
    fail "No build for $TARGET in $TAG. See https://github.com/$REPO/releases"

# --- check it is what was published ----------------------------------------

if curl -fsSL "$BASE/SHA256SUMS" -o "$WORK/SHA256SUMS" 2>/dev/null; then
    expected=$(grep " $ARCHIVE\$" "$WORK/SHA256SUMS" | awk '{print $1}')
    if [ -n "$expected" ]; then
        if command -v sha256sum >/dev/null 2>&1; then
            actual=$(sha256sum "$WORK/$ARCHIVE" | awk '{print $1}')
        else
            actual=$(shasum -a 256 "$WORK/$ARCHIVE" | awk '{print $1}')
        fi
        [ "$expected" = "$actual" ] || fail "Checksum mismatch. Not installing."
        say "  checksum   ok"
    fi
fi

# Signed through Sigstore and recorded in a public log, so this says the binary came
# from that repository's release workflow and not from somewhere else.
if command -v gh >/dev/null 2>&1; then
    if gh attestation verify "$WORK/$ARCHIVE" --repo "$REPO" >/dev/null 2>&1; then
        say "  provenance ok"
    else
        say "  provenance could not be verified - continuing, but be aware"
    fi
fi

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

# --- the GitHub CLI ---------------------------------------------------------

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

if command -v gh >/dev/null 2>&1; then
    if gh auth status >/dev/null 2>&1; then
        say "  github     signed in"
    elif confirm "You are not signed in to GitHub. Sign in now?"; then
        # Interactive on purpose: a device flow against access you already have.
        gh auth login < /dev/tty || true
    else
        say "  github     not signed in - run: gh auth login"
    fi
else
    say "The GitHub CLI is needed. Install it, then run: gh auth login"
    say "  https://cli.github.com"
fi

# --- completion -------------------------------------------------------------

START="# >>> deplyd completions >>>"
END="# <<< deplyd completions <<<"

case "${SHELL:-}" in
    */zsh)  rc="$HOME/.zshrc"; shell=zsh ;;
    */fish) rc="$HOME/.config/fish/config.fish"; shell=fish ;;
    */bash) rc="$HOME/.bashrc"; shell=bash ;;
    *)      rc=""; shell="" ;;
esac

if [ -n "$rc" ]; then
    mkdir -p "$(dirname "$rc")"
    [ -f "$rc" ] || : > "$rc"
    # Drop a previous block before writing this one, so reinstalling does not stack up.
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
