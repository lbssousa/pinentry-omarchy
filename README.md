# pinentry-omarchy

A GnuPG [pinentry](https://www.gnupg.org/related_software/pinentry/) for
[Omarchy](https://omarchy.org/) that asks for PINs and passphrases in the
same overlay dialog as the shell's polkit agent: same layer, scrim, card,
fonts, `[polkit]` theme colours and failure shake, following theme changes
live.

## How it works

```
gpg-agent ──Assuan (stdin/stdout)──▶ pinentry-omarchy (Rust)
                                        │ one NDJSON line each way
                                        ▼
            $XDG_RUNTIME_DIR/pinentry-omarchy.sock
                                        │
                  plugin lbssousa.pinentry (service) inside omarchy-shell
                                        │
                                  overlay dialog
```

- **`pinentry-omarchy`** (`src/`) speaks the pinentry Assuan protocol to
  gpg-agent: `GETPIN` (with `SETREPEAT`), `CONFIRM` (incl.
  `--one-button`), `MESSAGE`, the `SET*` texts and labels, `SETTIMEOUT`,
  `GETINFO`. For each prompt it connects to the plugin's socket (checking
  that the peer runs as the same user), sends the request and relays the
  answer.
- **The plugin** (`plugin/`) is a third-party Omarchy shell plugin of kind
  `service`. It reuses the shell's own `qs.Commons` (`Color`, `Style`,
  `Border`) and `qs.Ui` (`BorderSurface`, `Button`) instead of copying
  styles. One request is served at a time; a second one gets `busy`. A
  client that disconnects (gpg-agent killing the pinentry, for example)
  closes its dialog.
- **Fallback:** if there's no Wayland session or the plugin isn't
  listening when the pinentry starts, it `exec`s `pinentry-gnome3` (or
  `pinentry-curses` without a display) with the same arguments, before
  saying anything to gpg-agent. `PINENTRY_OMARCHY_FALLBACK` picks another
  program. If the shell goes away in the middle of a session, the pending
  prompt counts as cancelled.

### Protocol between the two halves

Request: `{"v":1,"type":"getpin"|"confirm"|"message","title","desc","prompt","error","ok","cancel","notok","repeat","repeatError","timeout"}`
(empty fields are omitted; `repeat` is the second field's placeholder).

Response: `{"result":"ok"|"cancel"|"notok"|"timeout"|"busy"|"error","pin"?,"message"?}`

### Security notes

- The binary locks its memory (`mlockall`), disables core dumps, and keeps
  the PIN in zeroized buffers.
- The socket lives in the user's 0700 runtime directory, and the binary
  checks the listening peer's uid.
- Texts from gpg-agent are rendered as plain text, never rich text.
- Inside omarchy-shell, the PIN goes through a QML `TextInput` and the
  JavaScript heap, which can't be wiped. The fields are cleared as soon as
  the answer is sent. This is the same trade-off as the polkit dialog.

## Install

On Arch/Omarchy, build and install the package:

```sh
just pkg                                  # packaging/pinentry-omarchy-*.pkg.tar.zst
sudo pacman -U packaging/pinentry-omarchy-*.pkg.tar.zst
```

Then, per user:

```sh
ln -sfn /usr/share/pinentry-omarchy/plugin ~/.config/omarchy/plugins/lbssousa.pinentry
omarchy-shell shell enablePlugin lbssousa.pinentry '{}'
echo 'pinentry-program /usr/bin/pinentry-omarchy' >> ~/.gnupg/gpg-agent.conf
gpgconf --reload gpg-agent
```

[omarchy-setup](https://github.com/lbssousa/omarchy-setup) automates all
of this (`just pinentry`).

## Development

| Recipe | What it does |
|---|---|
| `just test` | `cargo test` (Assuan parsing + integration tests against a fake dialog) and clippy |
| `just dev` | Runs the dialog in a separate Quickshell instance (`dev/`, which links the shell's `Commons`/`Ui`) on its own socket |
| `just try` | Sends a sample `GETPIN` through the `just dev` instance |
| `just link` | Loads this checkout's plugin in the real shell (restarts omarchy-shell) |
| `just pkg` | Builds the Arch package |

omarchy-shell doesn't reload a `keepLoaded` service when its files change,
and its file watcher doesn't follow the plugin symlink. Iterate with
`just dev`, and restart the shell when you're done.
