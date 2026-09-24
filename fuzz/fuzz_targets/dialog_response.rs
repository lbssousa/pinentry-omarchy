//! Parsing the dialog's reply: any bytes must yield a response or an
//! error, never a panic. A PIN, when present, is the percent-decoding of
//! what was sent.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pinentry_omarchy::shell::{MAX_RESPONSE_BYTES, read_response};

fuzz_target!(|data: &[u8]| {
    let Ok(response) = read_response(&mut &data[..]) else {
        return;
    };
    let line_len = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    assert!(line_len <= MAX_RESPONSE_BYTES);
    if let Some(pin) = response.pin {
        // Percent-decoding only ever shrinks its input.
        assert!(pin.len() <= line_len);
    }
});
