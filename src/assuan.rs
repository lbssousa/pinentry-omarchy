use std::io::{self, Write};

use zeroize::Zeroizing;

// gpg-error codes with GPG_ERR_SOURCE_PINENTRY (5 << 24), as sent by the
// upstream pinentry programs.
const SOURCE_PINENTRY: u32 = 5 << 24;
pub const ERR_CANCELED: u32 = SOURCE_PINENTRY | 99;
pub const ERR_NOT_CONFIRMED: u32 = SOURCE_PINENTRY | 114;
pub const ERR_TIMEOUT: u32 = SOURCE_PINENTRY | 62;
pub const ERR_PIN_ENTRY: u32 = SOURCE_PINENTRY | 86;
pub const ERR_UNKNOWN_CMD: u32 = SOURCE_PINENTRY | 275;
pub const ERR_INV_VALUE: u32 = SOURCE_PINENTRY | 55;

// Assuan lines are limited to 1000 bytes including "D " and the newline.
const MAX_DATA_PER_LINE: usize = 1000 - 4;

/// Splits a request line into its (upper-cased) command and raw argument.
pub fn split_command(line: &[u8]) -> (String, &[u8]) {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let (cmd, rest) = match line.iter().position(|&b| b == b' ') {
        Some(i) => (&line[..i], &line[i + 1..]),
        None => (line, &line[line.len()..]),
    };
    let start = rest.iter().position(|&b| b != b' ').unwrap_or(rest.len());
    (
        String::from_utf8_lossy(cmd).to_ascii_uppercase(),
        &rest[start..],
    )
}

/// Decodes Assuan %XX escapes; invalid escapes are kept literally.
pub fn percent_decode(arg: &[u8]) -> String {
    let mut out = Vec::with_capacity(arg.len());
    let mut i = 0;
    while i < arg.len() {
        if arg[i] == b'%' && i + 2 < arg.len() {
            let hex = std::str::from_utf8(&arg[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(arg[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strips GTK-style mnemonic underscores from button labels ("_OK" -> "OK",
/// "__" -> "_").
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn percent_encode_into(data: &[u8], out: &mut Zeroizing<Vec<u8>>) {
    for &b in data {
        if b == b'%' || b == b'\r' || b == b'\n' || b < 0x20 {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

pub struct Writer<W: Write> {
    inner: W,
}

impl<W: Write> Writer<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn ok(&mut self, msg: Option<&str>) -> io::Result<()> {
        match msg {
            Some(m) => writeln!(self.inner, "OK {m}")?,
            None => writeln!(self.inner, "OK")?,
        }
        self.inner.flush()
    }

    pub fn err(&mut self, code: u32, msg: &str) -> io::Result<()> {
        writeln!(self.inner, "ERR {code} {msg}")?;
        self.inner.flush()
    }

    pub fn status(&mut self, keyword: &str) -> io::Result<()> {
        writeln!(self.inner, "S {keyword}")
    }

    /// Sends `data` as one or more "D" lines, escaping as Assuan requires.
    /// The encoded copy is wiped once written.
    pub fn data(&mut self, data: &[u8]) -> io::Result<()> {
        let mut encoded = Zeroizing::new(Vec::with_capacity(data.len() + 8));
        percent_encode_into(data, &mut encoded);
        let mut rest: &[u8] = &encoded;
        while !rest.is_empty() {
            let mut cut = rest.len().min(MAX_DATA_PER_LINE);
            // Never split inside a %XX escape.
            if cut < rest.len()
                && let Some(p) = rest[cut - 2..cut].iter().position(|&b| b == b'%')
            {
                cut = cut - 2 + p;
            }
            self.inner.write_all(b"D ")?;
            self.inner.write_all(&rest[..cut])?;
            self.inner.write_all(b"\n")?;
            rest = &rest[cut..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_commands() {
        assert_eq!(split_command(b"getpin\n"), ("GETPIN".into(), &b""[..]));
        assert_eq!(
            split_command(b"SETDESC  hello world\r\n"),
            ("SETDESC".into(), &b"hello world"[..])
        );
    }

    #[test]
    fn decodes_escapes() {
        assert_eq!(percent_decode(b"line%0Anext%25"), "line\nnext%");
        assert_eq!(percent_decode(b"100%"), "100%");
        assert_eq!(percent_decode(b"%zz%4"), "%zz%4");
        assert_eq!(percent_decode(b"La%C3%A9rcio"), "Laércio");
    }

    #[test]
    fn strips_mnemonics() {
        assert_eq!(strip_mnemonic("_OK"), "OK");
        assert_eq!(strip_mnemonic("Do_n't"), "Don't");
        assert_eq!(strip_mnemonic("a__b"), "a_b");
    }

    #[test]
    fn encodes_data_lines() {
        let mut out = Vec::new();
        Writer::new(&mut out).data(b"a%b\nc").unwrap();
        assert_eq!(out, b"D a%25b%0Ac\n");
    }

    #[test]
    fn splits_long_data_without_breaking_escapes() {
        let mut data = vec![b'x'; MAX_DATA_PER_LINE - 1];
        data.push(b'%');
        data.push(b'y');
        let mut out = Vec::new();
        Writer::new(&mut out).data(&data).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with('x'));
        assert_eq!(lines[1], "D %25y");
    }
}
