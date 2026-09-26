#!/bin/sh
# Installs deplyd on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/deplyd/main/install.sh | sh
#
# Downloads the release for your platform, checks it against the published
# checksums, verifies its provenance when gh is available, and puts the binary
# somewhere on your PATH. Nothing needs root.
#
# Environment:
#   DEPLYD_INSTALL_DIR   where to put it (default ~/.local/bin)
#   DEPLYD_VERSION       a tag to install (default the latest release)

set -eu

REPO="BertilVossebelt/deplyd"
INSTALL_DIR="${DEPLYD_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf '\n%s\n\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required and was not found."
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

# deplyd needs gh anyway, so the provenance check costs nobody an extra tool.
# Signed through Sigstore and recorded in a public log, so this says the binary
# came from that repository's release workflow and not from somewhere else.
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
say ""

# --- is it reachable --------------------------------------------------------

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        say "Run 'deplyd check' to see what it is allowed to do."
        ;;
    *)
        say "$INSTALL_DIR is not on your PATH. Add it:"
        say ""
        case "${SHELL:-}" in
            */zsh)  say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc && exec zsh" ;;
            */fish) say "  fish_add_path $INSTALL_DIR" ;;
            *)      say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc && exec bash" ;;
        esac
        ;;
esac

say ""
say "deplyd reads GitHub as you. If you have not already:  gh auth login"
