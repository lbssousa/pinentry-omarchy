//! A whole pinentry session: arbitrary Assuan commands from "gpg-agent",
//! and an arbitrary reply from "the dialog" to the first prompt.
//!
//! Input: the command script, then optionally a 0xFF byte followed by the
//! dialog's reply. Checks that everything sent back to gpg-agent is
//! well-formed Assuan, and that every request sent to the dialog is valid
//! JSON whose texts carry no control or bidi characters.

#![no_main]

use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::Once;

use libfuzzer_sys::fuzz_target;
use pinentry_omarchy::assuan::MAX_RESPONSE_LINE;
use pinentry_omarchy::session;
use pinentry_omarchy::shell::Dialog;

static NO_SHELL: Once = Once::new();

fn is_hidden_char(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fuzz_target!(|input: &[u8]| {
    // Prompts after the first open a new connection; make sure it can never
    // reach a real shell's dialog.
    NO_SHELL.call_once(|| {
        // SAFETY: runs once, before this process reads the environment
        // from any other thread.
        unsafe {
            std::env::set_var(
                "PINENTRY_OMARCHY_SOCKET",
                "/nonexistent/pinentry-omarchy-fuzz.sock",
            )
        };
    });

    let (script, reply) = match input.iter().position(|&b| b == 0xFF) {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => (input, &[][..]),
    };
    // More than the socket buffer holds would block the write below; the
    // client rejects anything past 64 KiB anyway.
    if reply.len() > 64 * 1024 {
        return;
    }

    // The dialog's end: its reply is already queued, then it stops writing,
    // so the session never blocks waiting for it.
    let (ours, mut theirs) = UnixStream::pair().unwrap();
    theirs.write_all(reply).unwrap();
    theirs.shutdown(Shutdown::Write).unwrap();

    let mut out = Vec::new();
    session::serve(Dialog::new(ours), script, &mut out).unwrap();

    // What gpg-agent got: complete lines, each a valid response kind, within
    // Assuan's line limit, with no raw control characters.
    assert!(out.ends_with(b"\n"));
    for line in out.split_inclusive(|&b| b == b'\n') {
        assert!(line.len() <= MAX_RESPONSE_LINE, "{} byte line", line.len());
        let body = &line[..line.len() - 1];
        assert!(
            [&b"OK"[..], b"ERR ", b"D ", b"S "]
                .iter()
                .any(|p| body.starts_with(p)),
            "unexpected line {:?}",
            String::from_utf8_lossy(line)
        );
        assert!(body.iter().all(|&b| b >= 0x20), "control byte in {line:?}");
    }

    // What the dialog got: at most one request (the first prompt), as JSON,
    // with sanitized texts.
    // Closing the session's end with part of the reply unread resets the
    // connection; what was already queued for us is still delivered first.
    let mut sent = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match theirs.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => sent.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::ConnectionReset => break,
            Err(e) => panic!("reading the requests: {e}"),
        }
    }
    for request in String::from_utf8(sent).unwrap().lines() {
        let request: serde_json::Value = serde_json::from_str(request).unwrap();
        assert_eq!(request["v"], 2);
        for (key, value) in request.as_object().unwrap() {
            if let Some(text) = value.as_str() {
                assert!(!text.chars().any(is_hidden_char), "{key}: {text:?}");
            }
        }
    }
});
