use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::unistd::getuid;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::assuan::percent_decode_into;
use crate::request::Request;

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

// How long past the dialog's own timeout we wait for its answer.
const TIMEOUT_GRACE_SECS: u64 = 5;

pub fn socket_path() -> PathBuf {
    if let Some(p) = env::var_os("PINENTRY_OMARCHY_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", getuid())));
    runtime.join("pinentry-omarchy.sock")
}

/// Connects and makes sure the listening end belongs to this user.
pub fn connect() -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket_path())?;
    let creds = getsockopt(&stream, PeerCredentials).map_err(io::Error::from)?;
    if creds.uid() != getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pinentry socket is owned by another user",
        ));
    }
    Ok(stream)
}

/// The dialog's answer. `pin` arrives percent-encoded (UTF-8), so the JSON
/// never needs unescaping and serde borrows it straight from the zeroized
/// read buffer instead of copying it.
#[derive(Deserialize)]
struct RawResponse<'a> {
    result: &'a str,
    #[serde(borrow)]
    pin: Option<&'a str>,
    message: Option<String>,
}

pub struct Response {
    pub result: String,
    pub pin: Option<Zeroizing<Vec<u8>>>,
    pub message: Option<String>,
}

/// Connection to the dialog plugin. The plugin answers one request per
/// connection and then closes it.
pub struct Dialog {
    // The connection that proved the plugin is up; used for the first
    // request so that check doesn't cost an extra, empty connection.
    pending: Option<UnixStream>,
}

impl Dialog {
    pub fn new(first: UnixStream) -> Self {
        Self {
            pending: Some(first),
        }
    }

    /// Sends one request and waits for the answer. Dropping the stream
    /// without an answer (e.g. when gpg-agent kills us) closes the dialog.
    pub fn ask(&mut self, request: &Request<'_>, timeout_secs: u64) -> io::Result<Response> {
        let stream = match self.pending.take() {
            Some(stream) => stream,
            None => connect()?,
        };
        // The dialog enforces its timeout itself; this only guards against
        // a shell that stops answering.
        let read_timeout = (timeout_secs > 0)
            .then(|| Duration::from_secs(timeout_secs.saturating_add(TIMEOUT_GRACE_SECS)));
        ask_on(stream, request, read_timeout)
    }
}

fn ask_on(
    mut stream: UnixStream,
    request: &Request<'_>,
    read_timeout: Option<Duration>,
) -> io::Result<Response> {
    stream.set_read_timeout(read_timeout)?;
    let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    read_response(&mut stream)
}

/// Reads one response line into a fixed, zeroized buffer (no BufReader and
/// no reallocation, so no stray copies of the PIN) and parses it.
pub fn read_response(stream: &mut impl Read) -> io::Result<Response> {
    let mut buf = Zeroizing::new(vec![0u8; MAX_RESPONSE_BYTES]);
    let mut len = 0;
    let end = loop {
        if let Some(i) = buf[..len].iter().position(|&b| b == b'\n') {
            break i;
        }
        if len == buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pinentry dialog response too long",
            ));
        }
        match stream.read(&mut buf[len..]) {
            Ok(0) if len == 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "pinentry dialog went away",
                ));
            }
            Ok(0) => break len,
            Ok(n) => len += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    };
    let raw: RawResponse<'_> = serde_json::from_slice(&buf[..end]).map_err(io::Error::other)?;
    let pin = raw.pin.map(|encoded| {
        let mut pin = Zeroizing::new(Vec::with_capacity(encoded.len()));
        percent_decode_into(encoded.as_bytes(), &mut pin);
        pin
    });
    Ok(Response {
        result: raw.result.to_string(),
        pin,
        message: raw.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn respond_with(bytes: Vec<u8>) -> io::Result<Response> {
        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        let writer = thread::spawn(move || {
            let _ = theirs.write_all(&bytes);
        });
        let result = read_response(&mut ours);
        writer.join().unwrap();
        result
    }

    fn encode_like_the_plugin(pin: &str) -> String {
        // encodeURIComponent leaves only A-Z a-z 0-9 - _ . ! ~ * ' ( ) as is.
        let mut out = String::new();
        for &b in pin.as_bytes() {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    #[test]
    fn decodes_tricky_pins_exactly() {
        for pin in [
            "123456",
            "a%b\"c\\d\ne",
            "sénha çom acentos",
            "🔑🗝️",
            "%25%",
            " ",
        ] {
            let line = format!(
                "{{\"result\":\"ok\",\"pin\":\"{}\"}}\n",
                encode_like_the_plugin(pin)
            );
            let r = respond_with(line.into_bytes()).unwrap();
            assert_eq!(r.result, "ok");
            assert_eq!(
                r.pin.as_deref().map(Vec::as_slice),
                Some(pin.as_bytes()),
                "{pin:?}"
            );
        }
    }

    #[test]
    fn keeps_message_and_missing_pin() {
        let r = respond_with(br#"{"result":"error","message":"boom"}"#.to_vec()).unwrap();
        assert_eq!(r.result, "error");
        assert!(r.pin.is_none());
        assert_eq!(r.message.as_deref(), Some("boom"));
    }

    #[test]
    fn accepts_a_response_closed_without_newline() {
        let r = respond_with(br#"{"result":"cancel"}"#.to_vec()).unwrap();
        assert_eq!(r.result, "cancel");
    }

    #[test]
    fn rejects_oversized_response() {
        let err = respond_with(vec![b'x'; MAX_RESPONSE_BYTES + 10])
            .err()
            .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_invalid_json_and_escaped_pins() {
        assert!(respond_with(b"not json\n".to_vec()).is_err());
        // An escaped pin can't be borrowed: refusing it keeps unescaping
        // (and its unzeroized scratch copy) out of the picture.
        assert!(respond_with(br#"{"result":"ok","pin":"a\"b"}"#.to_vec()).is_err());
    }

    #[test]
    fn reports_a_vanished_dialog() {
        let err = respond_with(Vec::new()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn times_out_when_the_dialog_hangs() {
        let (mut ours, _theirs) = UnixStream::pair().unwrap();
        ours.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let err = read_response(&mut ours).err().unwrap();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }
}
