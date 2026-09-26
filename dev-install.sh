#!/bin/sh
# Builds the tree and puts that build on PATH for this terminal only. Close the
# terminal and it is gone: nothing is written to your rc file, your PATH, or the
# directory a real install lives in.
#
# Source it. A script cannot change the PATH of the shell that ran it, so without
# the leading dot the build lands and nothing points at it.
#
#   . ./dev-install.sh              build, then use it here
#   . ./dev-install.sh --release    the release profile, for a realistic binary
#   . ./dev-install.sh --persist    install it for real, to test the uninstaller
#   . ./dev-install.sh --revert     undo a --persist
#
# This is not the installer and never will be. install.sh takes a published release
# and refuses anything it cannot verify; a local build is neither, so staging one is
# a separate job with a separate name.
#
# Nothing here sets -e on purpose. Sourced, that would be your shell it was set on,
# and the next command that returned non-zero would close the terminal.

deplyd_dev() {
    # Sourced, so anything not made local is left behind in the caller's shell.
    local profile persist revert argument root install_dir built destination
    profile=debug
    persist=""
    revert=""
    for argument in "$@"; do
        case "$argument" in
            --release) profile=release ;;
            --persist) persist=1 ;;
            --revert)  revert=1 ;;
            *) printf '\nUnknown option: %s\nUsage: . ./dev-install.sh [--release] [--persist] [--revert]\n\n' \
                   "$argument" >&2
               return 1 ;;
        esac
    done

    # Sourced, $0 is the shell rather than this file, so the usual dirname does not
    # work. Each shell keeps the path somewhere else, and the last resort is asking
    # whether the working directory looks like the repository.
    if [ -n "${BASH_SOURCE:-}" ]; then
        root=$(dirname -- "$BASH_SOURCE")
    elif [ -n "${ZSH_VERSION:-}" ]; then
        # Through eval so that no other shell has to parse zsh's spelling.
        root=$(dirname -- "$(eval 'printf %s "${(%):-%x}"')")
    else
        root=$PWD
    fi
    root=$(CDPATH= cd -- "$root" && pwd) || return 1

    if [ ! -f "$root/install.sh" ] || [ ! -f "$root/Cargo.toml" ]; then
        printf '\nRun this from the deplyd repository: . ./dev-install.sh\n\n' >&2
        return 1
    fi

    # --- the persistent kind, for working on the installer -----------------

    if [ -n "$persist" ] || [ -n "$revert" ]; then
        # In a subshell: install.sh sets -e, and that is not something to hand to
        # an interactive shell.
        install_dir=$(
            set -eu
            set --
            DEPLYD_SOURCE_ONLY=1 . "$root/install.sh"
            [ -n "$revert" ] && uninstall >&2
            printf %s "$INSTALL_DIR"
        ) || return 1

        if [ -n "$revert" ]; then
            printf 'Now reinstall the published release:\n'
            printf '  curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh\n\n'
            return 0
        fi
    fi

    # --- build -------------------------------------------------------------

    printf '\nBuilding deplyd (%s)\n' "$profile"
    if [ "$profile" = release ]; then
        (cd "$root" && cargo build --release) || return 1
    else
        (cd "$root" && cargo build) || return 1
    fi

    built="$root/target/$profile/deplyd"
    [ -f "$built" ] || { printf '\nNo binary at %s\n\n' "$built" >&2; return 1; }

    # Under target, so it is gitignored and cargo clean takes it. A real install
    # would go to ~/.local/bin; this deliberately does not.
    if [ -n "$persist" ]; then
        destination=$install_dir
    else
        destination="$root/target/dev-bin"
    fi
    mkdir -p "$destination" || return 1

    cp "$built" "$destination/deplyd" || return 1
    chmod +x "$destination/deplyd"
    rm -f "$destination/dp"
    ln -s deplyd "$destination/dp" 2>/dev/null || cp "$destination/deplyd" "$destination/dp"

    printf '\n  built      %s\n' "$destination/deplyd"
    printf '  dp         short name for deplyd\n'

    if [ -n "$persist" ]; then
        # --- the parts that outlive the terminal ---------------------------
        (
            set -eu
            set --
            DEPLYD_SOURCE_ONLY=1 . "$root/install.sh"
            [ -n "$rc" ] || exit 0
            mkdir -p "$(dirname "$rc")"
            [ -f "$rc" ] || : > "$rc"
            strip_completions
            "$destination/deplyd" completions "$shell" >> "$rc"
            printf '  completion added to %s\n' "$rc"
            case ":$PATH:" in
                *":$destination:"*) ;;
                *)
                    if [ "$shell" = fish ]; then
                        printf '%s\n' "fish_add_path $destination" >> "$rc"
                    else
                        printf '%s\n' "export PATH=\"$destination:\$PATH\"" >> "$rc"
                    fi
                    printf '  path       added to %s\n' "$rc"
                    ;;
            esac
        ) || return 1

        printf '\nInstalled %s, and it will still be here tomorrow.\n' "$("$destination/deplyd" --version)"
        printf '  ./install.sh --uninstall     test the uninstaller against it\n'
        printf '  . ./dev-install.sh --revert  take it back off\n\n'
        return 0
    fi

    # --- this terminal only -------------------------------------------------

    # Prepended, and any earlier run of this dropped first, so sourcing twice does
    # not stack up and the build being tried is always the one in front.
    PATH=$(printf %s "$PATH" | tr ':' '\n' | grep -vxF "$destination" | tr '\n' ':' | sed 's/:$//')
    PATH="$destination:$PATH"
    export PATH

    # Into this shell, so completion works for the rest of the terminal without an
    # rc file ever being touched. fish cannot read a POSIX script, so it is on its
    # own here.
    if [ -n "${ZSH_VERSION:-}" ]; then
        eval "$("$destination/deplyd" completions zsh)" 2>/dev/null
    elif [ -n "${BASH_VERSION:-}" ]; then
        eval "$("$destination/deplyd" completions bash)" 2>/dev/null
    fi

    printf '\nUsing %s in this terminal only.\n' "$("$destination/deplyd" --version)"
    printf '  dp status       try it\n'
    printf '  close this terminal and nothing of it is left\n\n'
}

deplyd_dev "$@"
unset -f deplyd_dev
