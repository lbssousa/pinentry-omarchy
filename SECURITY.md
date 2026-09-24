# Security policy

## Supported versions

Only the latest release on `main` gets security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/lbssousa/pinentry-omarchy/security/advisories/new),
not in a public issue. Include the version (`pinentry-omarchy`'s
`GETINFO version`, or `pacman -Q pinentry-omarchy`), what you did, and
what happened. You should get an answer within a week.

## Threat model

pinentry-omarchy asks for PINs and passphrases on behalf of gpg-agent.
The OpenPGP card's PIN (e.g. a Yubikey's) is the main asset. A PIN that
reaches the card corrupted also costs something: each wrong PIN uses up
one of a limited number of tries.

**In scope:**

- Leaking the PIN: to other users, to swap or core dumps, or through
  buffers that outlive the prompt.
- Corrupting the PIN between the dialog and gpg-agent (encoding, framing,
  version mismatches).
- Text from outside, such as a key's user ID in `SETDESC` (possibly
  through a forwarded agent), that disguises what the dialog shows or
  injects Assuan protocol lines.
- Denial of service against the pinentry binary by malformed input from
  the dialog.

**Out of scope, by design:**

- **Processes running as the same user.** They can already rewrite
  `gpg-agent.conf`, open this dialog themselves, or draw a look-alike
  layer-shell overlay. Same-uid is the trust boundary, as it is for
  gpg-agent itself. The socket only accepts peers with the user's uid.
- **The PIN inside omarchy-shell.** It passes through a QML `TextInput`
  and the JavaScript heap, which can't be wiped. The fields are cleared
  right after answering. This is the same exposure as the shell's polkit
  dialog.
- **A hung shell.** With no `SETTIMEOUT`, a shell that accepts the
  connection but never answers keeps the pinentry waiting. gpg-agent (or
  the user, with Ctrl-C) can still cancel it.
- **Memory pressure on the shell.** A same-uid client can send an endless
  request line to the plugin's socket.

## Hardening in place

- `mlockall`, non-dumpable process (no core dumps, no same-uid ptrace).
- Zeroized buffers for every copy of the PIN in the binary, and
  unbuffered writes to gpg-agent.
- The PIN travels percent-encoded, so it's never JSON-unescaped into
  scratch memory. Protocol versioning makes mismatched halves fail closed.
- Display texts are stripped of control characters and bidirectional
  overrides, and rendered as plain text.
- Response lines to gpg-agent can't contain control characters.
- Bounded input: 1000-byte Assuan lines, 64 KiB dialog replies, and
  timeouts capped at one day.
- The Assuan session, the dialog-reply parser and the `D`-line encoder are
  fuzzed with cargo-fuzz (`fuzz/`): on every pull request that touches the
  code, and for longer every week.
- Releases are built from GPG-signed commits.
  [omarchy-setup](https://github.com/lbssousa/omarchy-setup) verifies the
  signature before building.
