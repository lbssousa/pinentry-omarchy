use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::unistd::getuid;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::request::Request;

const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

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

#[derive(Deserialize)]
pub struct Response {
    pub result: String,
    pub pin: Option<Zeroizing<String>>,
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
        ask_on(stream, request, timeout_secs)
    }
}

fn ask_on(
    mut stream: UnixStream,
    request: &Request<'_>,
    timeout_secs: u64,
) -> io::Result<Response> {
    if timeout_secs > 0 {
        // The dialog enforces the timeout itself; this only guards against a
        // shell that stops answering.
        stream.set_read_timeout(Some(Duration::from_secs(timeout_secs + 5)))?;
    }
    let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;

    let mut reader = BufReader::new(stream.take(MAX_RESPONSE_BYTES));
    let mut buf = Zeroizing::new(Vec::new());
    reader.read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "pinentry dialog went away",
        ));
    }
    serde_json::from_slice(&buf).map_err(io::Error::other)
}
