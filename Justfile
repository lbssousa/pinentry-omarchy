set shell := ["bash", "-uc"]

dev_socket := env("XDG_RUNTIME_DIR", "/run/user/1000") / "pinentry-omarchy-dev.sock"
plugin_link := env("HOME") / ".config/omarchy/plugins/lbssousa.pinentry"

default:
    @just --list

# Debug build.
build:
    cargo build

# Unit + integration tests, formatting and clippy.
test:
    cargo test --locked
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

# Protocol tests against the real QML plugin in a throwaway Quickshell
# instance (needs this Wayland session; dialogs flash on screen).
test-plugin:
    cargo test --locked --test plugin -- --ignored

# Fuzz one target (assuan_session, dialog_response, data_encoding) for
# `secs` seconds; needs `rustup toolchain install nightly` and
# `cargo install cargo-fuzz`. New inputs go to fuzz/corpus/, crashes to
# fuzz/artifacts/.
fuzz target="assuan_session" secs="60":
    mkdir -p fuzz/corpus/{{target}}
    cargo +nightly fuzz run {{target}} fuzz/corpus/{{target}} fuzz/seeds/{{target}} -- -max_total_time={{secs}} -max_len=8192

# Run the dialog in a separate Quickshell instance (dev/), listening on its
# own socket, so QML edits only need this restarted — not omarchy-shell.
dev:
    PINENTRY_OMARCHY_SOCKET={{dev_socket}} quickshell -p dev

# Ask for a PIN through the `just dev` instance and print the Assuan reply.
try: build
    printf 'SETTITLE pinentry-omarchy\nSETDESC Please enter the PIN%%0Ato unlock the card\nSETPROMPT PIN:\nSETERROR Bad PIN (2 tries left)\nGETPIN\nBYE\n' \
      | PINENTRY_OMARCHY_SOCKET={{dev_socket}} target/debug/pinentry-omarchy

# Load this checkout's plugin in the real shell (restarts omarchy-shell: a
# keepLoaded service survives plugin hot-reloads).
link:
    ln -sfn "$PWD/plugin" {{plugin_link}}
    omarchy-shell shell enablePlugin lbssousa.pinentry '{}'
    omarchy-restart-shell

# Build the Arch package into packaging/. --nodeps: cargo usually comes from
# rustup in ~/.cargo, which pacman doesn't know about.
pkg:
    cd packaging && makepkg -f --nodeps
