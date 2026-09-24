use std::io::{self, BufRead, Write};

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
pub const ERR_LINE_TOO_LONG: u32 = SOURCE_PINENTRY | 263;

/// Longest request line accepted, newline included (libassuan's
/// ASSUAN_LINELENGTH).
pub const MAX_LINE: usize = 1002;

/// Longest response line we send, newline included (Assuan's limit).
pub const MAX_RESPONSE_LINE: usize = 1000;

// "D " + data + newline must fit in one response line.
const MAX_DATA_PER_LINE: usize = MAX_RESPONSE_LINE - 3;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

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

pub enum LineRead {
    Line,
    TooLong,
    Eof,
}

/// Reads one line of at most `max` bytes (newline included) into `buf`.
/// A longer line is consumed up to its newline and reported as TooLong.
pub fn read_line_limited(
    input: &mut impl BufRead,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<LineRead> {
    buf.clear();
    let mut too_long = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if too_long {
                LineRead::TooLong
            } else if buf.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Line
            });
        }
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (&available[..=i], true),
            None => (available, false),
        };
        let used = chunk.len();
        if !too_long && buf.len() + used <= max {
            buf.extend_from_slice(chunk);
        } else {
            too_long = true;
            buf.clear();
        }
        input.consume(used);
        if done {
            return Ok(if too_long {
                LineRead::TooLong
            } else {
                LineRead::Line
            });
        }
    }
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes %XX escapes into `out`; invalid escapes are kept literally.
pub fn percent_decode_into(arg: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < arg.len() {
        if arg[i] == b'%'
            && i + 2 < arg.len()
            && let (Some(hi), Some(lo)) = (hex_value(arg[i + 1]), hex_value(arg[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(arg[i]);
        i += 1;
    }
}

/// Decodes Assuan %XX escapes into text (invalid UTF-8 becomes U+FFFD).
pub fn percent_decode(arg: &[u8]) -> String {
    let mut out = Vec::with_capacity(arg.len());
    percent_decode_into(arg, &mut out);
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

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Removes characters that could disguise what the dialog shows: control
/// characters (newlines and tabs are kept) and bidirectional overrides.
pub fn sanitize_display(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control() || is_bidi_control(c)))
        .collect()
}

/// Builds a response line `prefix` + free text + newline. Control
/// characters (CR/LF above all) become spaces, so the text can't start a
/// new line, and the text is cut at a character boundary so the line stays
/// within Assuan's limit.
fn response_line(prefix: &str, text: &str) -> String {
    let budget = MAX_RESPONSE_LINE - prefix.len() - 1;
    let mut line = String::with_capacity(prefix.len() + text.len().min(budget) + 1);
    line.push_str(prefix);
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if line.len() - prefix.len() + c.len_utf8() > budget {
            break;
        }
        line.push(c);
    }
    line.push('\n');
    line
}

fn percent_encode_into(data: &[u8], out: &mut Zeroizing<Vec<u8>>) {
    for &b in data {
        if b == b'%' || b < 0x20 {
            out.extend_from_slice(&[b'%', HEX[usize::from(b >> 4)], HEX[usize::from(b & 0xF)]]);
        } else {
            out.push(b);
        }
    }
}

/// Writes Assuan responses. Every line goes out in a single write, so an
/// unbuffered sink never holds a partial line (or a PIN) in a buffer.
pub struct Writer<W: Write> {
    inner: W,
}

impl<W: Write> Writer<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    fn line(&mut self, line: &[u8]) -> io::Result<()> {
        self.inner.write_all(line)?;
        self.inner.flush()
    }

    pub fn ok(&mut self, msg: Option<&str>) -> io::Result<()> {
        let line = match msg {
            Some(m) => response_line("OK ", m),
            None => "OK\n".to_string(),
        };
        self.line(line.as_bytes())
    }

    pub fn err(&mut self, code: u32, msg: &str) -> io::Result<()> {
        self.line(response_line(&format!("ERR {code} "), msg).as_bytes())
    }

    pub fn status(&mut self, keyword: &str) -> io::Result<()> {
        self.line(response_line("S ", keyword).as_bytes())
    }

    /// Sends `data` as one or more "D" lines, escaping as Assuan requires.
    /// Every copy made along the way is wiped.
    pub fn data(&mut self, data: &[u8]) -> io::Result<()> {
        let mut encoded = Zeroizing::new(Vec::with_capacity(data.len() * 3));
        percent_encode_into(data, &mut encoded);
        let mut line = Zeroizing::new(Vec::with_capacity(MAX_DATA_PER_LINE + 3));
        let mut rest: &[u8] = &encoded;
        while !rest.is_empty() {
            let mut cut = rest.len().min(MAX_DATA_PER_LINE);
            // Never split inside a %XX escape.
            if cut < rest.len()
                && let Some(p) = rest[cut - 2..cut].iter().position(|&b| b == b'%')
            {
                cut = cut - 2 + p;
            }
            line.clear();
            line.extend_from_slice(b"D ");
            line.extend_from_slice(&rest[..cut]);
            line.push(b'\n');
            self.line(&line)?;
            rest = &rest[cut..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_lines(data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        Writer::new(&mut out).data(data).unwrap();
        out.split_inclusive(|&b| b == b'\n')
            .map(<[u8]>::to_vec)
            .collect()
    }

    fn decode_data_lines(lines: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            let body = line
                .strip_prefix(b"D ")
                .unwrap()
                .strip_suffix(b"\n")
                .unwrap();
            percent_decode_into(body, &mut out);
        }
        out
    }

    #[test]
    fn splits_commands() {
        assert_eq!(split_command(b"getpin\n"), ("GETPIN".into(), &b""[..]));
        assert_eq!(
            split_command(b"SETDESC  hello world\r\n"),
            ("SETDESC".into(), &b"hello world"[..])
        );
        assert_eq!(split_command(b""), ("".into(), &b""[..]));
        assert_eq!(split_command(b"\n"), ("".into(), &b""[..]));
        assert_eq!(split_command(b"bye   \n"), ("BYE".into(), &b""[..]));
        assert_eq!(
            split_command(b"setprompt PIN:\n"),
            ("SETPROMPT".into(), &b"PIN:"[..])
        );
    }

    #[test]
    fn decodes_escapes() {
        assert_eq!(percent_decode(b"line%0Anext%25"), "line\nnext%");
        assert_eq!(percent_decode(b"line%0anext"), "line\nnext");
        assert_eq!(percent_decode(b"100%"), "100%");
        assert_eq!(percent_decode(b"50%4"), "50%4");
        assert_eq!(percent_decode(b"%zz%4"), "%zz%4");
        assert_eq!(percent_decode(b"%+1"), "%+1");
        assert_eq!(percent_decode(b"a%00b"), "a\0b");
        assert_eq!(percent_decode(b"La%C3%A9rcio"), "Laércio");
        assert_eq!(percent_decode(b"bad%FFutf8"), "bad\u{FFFD}utf8");
    }

    #[test]
    fn every_byte_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        let lines = data_lines(&all);
        for line in &lines {
            let body = &line[2..line.len() - 1];
            assert!(
                body.iter().all(|&b| b != b'\n' && b != b'\r' && b >= 0x20),
                "raw control byte in {body:?}"
            );
        }
        assert_eq!(decode_data_lines(&lines), all);
    }

    #[test]
    fn empty_data_writes_nothing() {
        assert!(data_lines(b"").is_empty());
    }

    #[test]
    fn data_lines_respect_the_limit_and_never_split_escapes() {
        for len in 990usize..=1010 {
            for pct in len.saturating_sub(6)..len {
                let mut data = vec![b'x'; len];
                data[pct] = b'%';
                let lines = data_lines(&data);
                for line in &lines {
                    assert!(
                        line.len() <= 1000,
                        "len {len} pct {pct}: {} bytes",
                        line.len()
                    );
                    let body = &line[2..line.len() - 1];
                    for (i, &b) in body.iter().enumerate() {
                        if b == b'%' {
                            assert!(i + 2 < body.len(), "len {len} pct {pct}: split escape");
                        }
                    }
                }
                assert_eq!(decode_data_lines(&lines), data, "len {len} pct {pct}");
            }
        }
    }

    #[test]
    fn response_text_cannot_inject_lines() {
        let mut out = Vec::new();
        let mut w = Writer::new(&mut out);
        w.err(ERR_PIN_ENTRY, "x\nD 1234\r\nOK\0").unwrap();
        w.ok(Some("a\nb")).unwrap();
        w.status("PIN_REPEATED\nD 1").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            "ERR 83886166 x D 1234  OK \nOK a b\nS PIN_REPEATED D 1\n"
        );
    }

    #[test]
    fn response_lines_fit_assuan_limit() {
        let mut out = Vec::new();
        let mut w = Writer::new(&mut out);
        w.err(ERR_PIN_ENTRY, &"x".repeat(5000)).unwrap();
        w.ok(Some(&"é".repeat(3000))).unwrap();
        w.status(&"🔑".repeat(1000)).unwrap();
        let text = String::from_utf8(out).unwrap();
        for line in text.split_inclusive('\n') {
            assert!(line.len() <= MAX_RESPONSE_LINE, "{} bytes", line.len());
            assert!(
                line.len() > MAX_RESPONSE_LINE - 4,
                "cut too early: {} bytes",
                line.len()
            );
        }
    }

    #[test]
    fn sanitizes_display_text() {
        assert_eq!(
            sanitize_display("evil\u{202E}gpj.exe\u{2066}x\u{2069}"),
            "evilgpj.exex"
        );
        assert_eq!(
            sanitize_display("a\u{7}b\u{1B}[31mc\u{85}d\u{7F}"),
            "ab[31mcd"
        );
        assert_eq!(
            sanitize_display("Laércio\n\tDe Sousa 🔑"),
            "Laércio\n\tDe Sousa 🔑"
        );
    }

    #[test]
    fn strips_mnemonics() {
        assert_eq!(strip_mnemonic("_OK"), "OK");
        assert_eq!(strip_mnemonic("Do_n't"), "Don't");
        assert_eq!(strip_mnemonic("a__b"), "a_b");
        assert_eq!(strip_mnemonic("end_"), "end");
        assert_eq!(strip_mnemonic("___"), "_");
        assert_eq!(strip_mnemonic(""), "");
    }

    fn read_all_lines(input: &[u8], max: usize) -> Vec<(&'static str, Vec<u8>)> {
        let mut reader = io::BufReader::with_capacity(7, input);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        loop {
            match read_line_limited(&mut reader, &mut buf, max).unwrap() {
                LineRead::Line => out.push(("line", buf.clone())),
                LineRead::TooLong => out.push(("too long", buf.clone())),
                LineRead::Eof => return out,
            }
        }
    }

    #[test]
    fn limits_line_length() {
        let lines = read_all_lines(b"abc\n0123456789\nxy\nlast", 5);
        assert_eq!(
            lines,
            [
                ("line", b"abc\n".to_vec()),
                ("too long", vec![]),
                ("line", b"xy\n".to_vec()),
                ("line", b"last".to_vec()),
            ]
        );
        assert_eq!(read_all_lines(b"abcd\n", 5), [("line", b"abcd\n".to_vec())]);
        assert_eq!(read_all_lines(b"abcde\n", 5), [("too long", vec![])]);
        assert_eq!(read_all_lines(b"abcdefgh", 5), [("too long", vec![])]);
        assert!(read_all_lines(b"", 5).is_empty());
    }
}
