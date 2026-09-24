//! Exercises the real QML plugin in a Quickshell instance built from dev/.
//! Needs a Wayland session with Omarchy installed (dev/ links the shell's
//! Commons and Ui), and briefly flashes dialogs on screen, so it is
//! ignored by default: run it with `just test-plugin`.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Shell(Child);

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_shell(socket: &Path) -> Shell {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("dev");
    let child = Command::new("quickshell")
        .arg("-p")
        .arg(dev)
        .env("PINENTRY_OMARCHY_SOCKET", socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("quickshell must be installed");
    let shell = Shell(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    while UnixStream::connect(socket).is_err() {
        assert!(
            Instant::now() < deadline,
            "the plugin never started listening"
        );
        thread::sleep(Duration::from_millis(100));
    }
    shell
}

fn send(socket: &Path, request: &[u8]) -> UnixStream {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(request).unwrap();
    stream
}

/// Reads until the plugin closes the connection, which it does after
/// every answer.
fn answer(mut stream: UnixStream, wait: Duration) -> serde_json::Value {
    stream.set_read_timeout(Some(wait)).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    assert!(out.ends_with('\n'), "unterminated answer {out:?}");
    serde_json::from_str(&out).unwrap()
}

fn ask(socket: &Path, request: &str) -> serde_json::Value {
    answer(send(socket, request.as_bytes()), Duration::from_secs(10))
}

#[test]
#[ignore = "needs a Wayland session and Omarchy's shell; run with `just test-plugin`"]
fn plugin_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("pinentry.sock");
    let _shell = start_shell(&socket);

    // Malformed requests are refused without opening a dialog.
    let r = ask(&socket, "not json\n");
    assert_eq!(
        (r["result"].as_str(), r["message"].as_str()),
        (Some("error"), Some("invalid JSON"))
    );
    let r = ask(&socket, "{\"v\":1,\"type\":\"getpin\"}\n");
    assert_eq!(
        r["message"], "unsupported request",
        "old protocol versions are refused"
    );
    let r = ask(&socket, "{\"v\":2,\"type\":\"frob\"}\n");
    assert_eq!(r["message"], "unsupported request");
    let huge = format!(
        "{{\"v\":2,\"type\":\"message\",\"desc\":\"{}\"}}\n",
        "x".repeat(70_000)
    );
    let r = ask(&socket, &huge);
    assert_eq!(r["message"], "request too large");

    // The dialog's own timeout answers and closes the connection.
    let r = ask(&socket, "{\"v\":2,\"type\":\"getpin\",\"timeout\":1}\n");
    assert_eq!(r["result"], "timeout");
    assert!(r.get("pin").is_none(), "no PIN without an OK");

    // One dialog at a time.
    let first = send(&socket, b"{\"v\":2,\"type\":\"message\",\"timeout\":2}\n");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        ask(&socket, "{\"v\":2,\"type\":\"message\"}\n")["result"],
        "busy"
    );
    assert_eq!(answer(first, Duration::from_secs(10))["result"], "timeout");

    // A client that goes away closes its dialog and frees the plugin.
    let gone = send(&socket, b"{\"v\":2,\"type\":\"getpin\",\"timeout\":60}\n");
    thread::sleep(Duration::from_millis(300));
    drop(gone);
    thread::sleep(Duration::from_millis(300));
    let r = ask(&socket, "{\"v\":2,\"type\":\"message\",\"timeout\":1}\n");
    assert_eq!(r["result"], "timeout", "the dialog was freed");

    // An absurd timeout is capped instead of overflowing into "now".
    let mut capped = send(
        &socket,
        b"{\"v\":2,\"type\":\"message\",\"timeout\":1e12}\n",
    );
    capped
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    let mut buf = [0u8; 64];
    let err = capped
        .read(&mut buf)
        .expect_err("the dialog should still be open");
    assert!(matches!(
        err.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut
    ));
    drop(capped);
}
