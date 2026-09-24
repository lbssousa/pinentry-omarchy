# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/). Releases from 0.2.1 on
are GPG-signed tags (see the README's "Releases" section). Earlier versions
were never tagged.

## [Unreleased]

### Security
- The crate now forbids `unsafe` code (`#![forbid(unsafe_code)]`).

### Added
- A contribution guide (`CONTRIBUTING.md`), including the rule that every
  change comes with tests.
- This changelog.

## [0.2.1] - 2026-09-24

### Fixed
- `OK`/`ERR`/`S` response lines could exceed Assuan's 1000-byte line limit
  when the dialog returned a long error message. gpg-agent would then
  reject the line. They're now cut to fit at a character boundary. Found
  by fuzzing.

### Added
- cargo-fuzz targets for the Assuan session, the dialog-reply parser and
  the `D`-line encoder (`fuzz/`, `just fuzz`). They run on every pull
  request that touches the code, and for longer every week.
- A "Releases" section in the README: releases are GPG-signed tags.

### Changed
- The protocol code moved into a library (`src/lib.rs`, `src/session.rs`)
  so the fuzz targets can drive it. The binary's behavior is unchanged.

## 0.2.0 - 2026-09-24

### Security
Found in an internal review; none of these issues had been reported
externally.
- **The PIN lingered in memory that was never wiped.** It was left in a
  `BufReader`'s buffer when reading the dialog's reply, in the standard
  output buffer after the `D` line was written, in serde_json's scratch
  buffer when unescaping, and in temporary hex strings. Every copy now
  lives in a zeroized buffer, and responses are written to gpg-agent
  unbuffered.
- **Assuan injection:** an error message from the dialog was echoed into an
  `ERR` line without escaping, so a message containing newlines could
  inject `D`/`OK` lines into what gpg-agent read. Response lines are now
  free of control characters.
- **Display spoofing:** texts from gpg-agent (for example a key's user ID
  in `SETDESC`) could use control characters or bidirectional overrides to
  disguise what the dialog showed. They're now stripped.
- **Unbounded input:** `SETTIMEOUT` could overflow (in the binary and in the
  plugin's timer), and gpg-agent lines had no length limit. Timeouts are now
  capped at one day, and lines at Assuan's 1000 bytes.
- The fallback pinentries are now run by absolute path.

### Changed
- **Protocol v2** between the binary and the plugin: the PIN travels
  percent-encoded. The plugin refuses any other protocol version, so
  mismatched halves fail closed instead of sending the card a misread PIN.
  After upgrading, restart omarchy-shell.

### Added
- `SECURITY.md` (threat model, private vulnerability reporting).
- Much broader unit and integration tests, and protocol tests against the
  real QML plugin (`just test-plugin`).
- GitHub security tooling: CI, CodeQL, cargo-deny, OpenSSF Scorecard,
  Dependabot.

## 0.1.1 - 2026-09-24

### Fixed
- omarchy-shell logged a `PeerClosedError` warning for every prompt. The
  startup availability check now reuses its connection for the first
  prompt, and the plugin closes each connection itself after answering.

## 0.1.0 - 2026-09-24

### Added
- First release. The Rust pinentry speaks Assuan to gpg-agent, and the
  omarchy-shell plugin draws the prompt in the polkit agent's overlay
  dialog: `GETPIN` (with `SETREPEAT`), `CONFIRM`, `MESSAGE`. Without the
  shell it falls back to pinentry-gnome3 or pinentry-curses.
- Arch package (`packaging/PKGBUILD`).

[Unreleased]: https://github.com/lbssousa/pinentry-omarchy/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/lbssousa/pinentry-omarchy/releases/tag/v0.2.1
