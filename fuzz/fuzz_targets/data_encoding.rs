//! "D" lines: whatever the PIN, the lines sent to gpg-agent fit Assuan's
//! limit, carry no raw control bytes, never split an escape, and decode
//! back to exactly the PIN.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pinentry_omarchy::assuan::{MAX_RESPONSE_LINE, Writer, percent_decode_into};

fuzz_target!(|pin: &[u8]| {
    let mut out = Vec::new();
    Writer::new(&mut out).data(pin).unwrap();

    let mut decoded = Vec::new();
    for line in out.split_inclusive(|&b| b == b'\n') {
        assert!(line.len() <= MAX_RESPONSE_LINE, "{} byte line", line.len());
        let body = line
            .strip_prefix(b"D ")
            .and_then(|l| l.strip_suffix(b"\n"))
            .expect("a complete D line");
        assert!(body.iter().all(|&b| b >= 0x20), "raw control byte");
        for (i, &b) in body.iter().enumerate() {
            if b == b'%' {
                assert!(i + 2 < body.len(), "escape split across lines");
            }
        }
        percent_decode_into(body, &mut decoded);
    }
    assert_eq!(decoded, pin);
});
