# Contributing to pinentry-omarchy

Thanks for helping. This project handles PINs and passphrases, so changes
are held to a few firm rules.

## Reporting bugs and asking for features

- **Bugs and feature requests:** open a
  [GitHub issue](https://github.com/lbssousa/pinentry-omarchy/issues).
  Include the version (`pacman -Q pinentry-omarchy`), your Omarchy version
  (`pacman -Q omarchy`), what you did, what you expected, and what happened.
- **Security vulnerabilities:** **never** report them in a public issue.
  Follow [SECURITY.md](SECURITY.md) and use GitHub's private vulnerability
  reporting.

Issues are usually answered within a week.

## Making a change

1. Fork the repository and create a branch from `main`.
2. Make your change. Keep it focused: one topic per pull request.
3. **Add or update tests.** This is a requirement, not a suggestion. New
   functionality must come with tests that exercise it, and bug fixes with a
   test that fails without the fix. Pick the level that fits:
   - unit tests next to the code (`#[cfg(test)]` modules in `src/`);
   - integration tests against a fake dialog (`tests/assuan.rs`);
   - protocol tests against the real QML plugin (`tests/plugin.rs`, run with
     `just test-plugin` inside a Wayland session);
   - a fuzz target or seed in `fuzz/`, when the change touches the Assuan
     parser, the dialog reply parser or the `D`-line encoding.
4. Run the checks locally:
   ```sh
   just test            # unit + integration tests, rustfmt, clippy -D warnings
   cargo deny check     # dependency advisories, licenses, sources
   just fuzz assuan_session 60   # if you touched protocol code (needs nightly + cargo-fuzz)
   ```
5. **Sign your commits** (`git commit -S`). `main` only accepts signed
   commits.
6. Open a pull request against `main` that says what changed and why. If it
   changes behavior users can see, add an entry under "Unreleased" in
   [CHANGELOG.md](CHANGELOG.md).

## What gets merged

`main` is protected. A pull request can only be merged, and only as a
squash merge, when:

- CI passes: tests, rustfmt and clippy with warnings treated as errors;
- CodeQL finds no new issues;
- the branch is up to date with `main`.

The fuzz and cargo-deny workflows also run on pull requests that touch code
or dependencies, and must pass too.

### Coding rules

- **No `unsafe`.** The crate is `#![forbid(unsafe_code)]`, and system calls
  go through `nix`'s safe wrappers. Only test harnesses in `fuzz/` may use
  `unsafe`, with a `// SAFETY:` comment.
- **Secrets:** anything that holds a PIN lives in a `zeroize::Zeroizing`
  buffer. Never log it, never `Debug`-print it, and never copy it into a
  buffer that isn't wiped.
- **Untrusted text:** text that reaches the dialog goes through
  `assuan::sanitize_display` and is rendered as `Text.PlainText`. Text that
  goes back to gpg-agent goes through `assuan::Writer`.
- **Protocol:** changes to the binary ↔ plugin protocol bump
  `request::PROTOCOL_VERSION` and the plugin's `protocolVersion` together.

## Releases

Releases are GPG-signed tags made by the maintainer. See the "Releases"
section of the [README](README.md#releases).

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
