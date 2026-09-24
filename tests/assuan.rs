use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_pinentry-omarchy");

/// What the fake dialog does with the next request.
enum Answer {
    /// Replies with this line (a newline is added) and closes, like the plugin.
    Line(String),
    /// Writes these bytes as they are and closes.
    Raw(Vec<u8>),
    /// Reads the request and never answers.
    Hang,
}

fn line(json: &str) -> Answer {
    Answer::Line(json.to_string())
}

fn pin_reply(pin: &str) -> Answer {
    let mut encoded = String::new();
    for &b in pin.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    Answer::Line(format!(r#"{{"result":"ok","pin":"{encoded}"}}"#))
}

struct FakeDialog {
    socket: PathBuf,
    requests: mpsc::Receiver<serde_json::Value>,
    connections: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

/// Fake shell plugin: answers each request with the next scripted answer,
/// reports the requests it saw and counts connections. Connections that
/// send nothing (a session that never prompts) are only counted.
fn fake_dialog(answers: Vec<Answer>) -> FakeDialog {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s");
    let listener = UnixListener::bind(&socket).unwrap();
    let (tx, requests) = mpsc::channel();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    thread::spawn(move || {
        let mut answers = answers.into_iter();
        let mut hung = Vec::new();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            let mut request = String::new();
            if BufReader::new(&stream).read_line(&mut request).unwrap() == 0 {
                continue;
            }
            let _ = tx.send(serde_json::from_str(&request).unwrap());
            match answers.next() {
                Some(Answer::Line(l)) => {
                    let _ = writeln!(stream, "{l}");
                }
                Some(Answer::Raw(bytes)) => {
                    let _ = stream.write_all(&bytes);
                }
                Some(Answer::Hang) => hung.push(stream),
                None => {}
            }
        }
    });
    FakeDialog {
        socket,
        requests,
        connections,
        _dir: dir,
    }
}

fn run_with(socket: &Path, script: &[u8]) -> Output {
    let mut child = Command::new(BIN)
        .env("PINENTRY_OMARCHY_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(script).unwrap();
    child.wait_with_output().unwrap()
}

/// Runs a script and returns the response lines after the greeting.
fn run(dialog: &FakeDialog, script: &str) -> Vec<String> {
    let out = run_with(&dialog.socket, script.as_bytes());
    assert!(out.status.success(), "exit status {:?}", out.status);
    let text = String::from_utf8(out.stdout).unwrap();
    let mut lines = text.lines().map(str::to_string);
    let greeting = lines.next().unwrap();
    assert!(greeting.starts_with("OK Pleased to meet you"), "{greeting}");
    lines.collect()
}

fn decode_data(lines: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for l in lines.iter().filter_map(|l| l.strip_prefix("D ")) {
        let b = l.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&l[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    out
}

#[test]
fn getpin_returns_escaped_pin() {
    let d = fake_dialog(vec![pin_reply("ab%c")]);
    let lines = run(
        &d,
        "SETDESC Hello%0AWorld\nSETPROMPT PIN:\nSETOK _Unlock\nGETPIN\nBYE\n",
    );
    assert_eq!(
        lines,
        ["OK", "OK", "OK", "D ab%25c", "OK", "OK closing connection"]
    );

    let req = d.requests.recv().unwrap();
    assert_eq!(req["v"], 2);
    assert_eq!(req["type"], "getpin");
    assert_eq!(req["desc"], "Hello\nWorld");
    assert_eq!(req["prompt"], "PIN");
    assert_eq!(req["ok"], "Unlock");
}

#[test]
fn tricky_and_long_pins_survive_exactly() {
    let long: String = "ç%\"\\\n🔑x".repeat(120); // well over one 1000-byte D line
    for pin in ["a\"b\\c", "sénha 🔑", "%0A%", "tab\there", long.as_str()] {
        let d = fake_dialog(vec![pin_reply(pin)]);
        let lines = run(&d, "GETPIN\n");
        assert_eq!(lines.last().map(String::as_str), Some("OK"), "{pin:?}");
        for l in &lines {
            assert!(l.len() < 1000, "line of {} bytes", l.len());
        }
        assert_eq!(decode_data(&lines), pin.as_bytes(), "{pin:?}");
    }
}

#[test]
fn empty_pin_sends_no_data_line() {
    let d = fake_dialog(vec![line(r#"{"result":"ok","pin":""}"#)]);
    assert_eq!(run(&d, "GETPIN\n"), ["OK"]);
}

#[test]
fn cancel_timeout_and_repeat() {
    let d = fake_dialog(vec![
        line(r#"{"result":"cancel"}"#),
        line(r#"{"result":"timeout"}"#),
        pin_reply("x"),
        line(r#"{"result":"ok"}"#),
    ]);
    let lines = run(
        &d,
        "SETERROR Bad PIN\nGETPIN\nGETPIN\nSETREPEAT Again:\nGETPIN\nGETPIN\n",
    );
    assert_eq!(
        lines,
        [
            "OK",
            "ERR 83886179 Operation cancelled",
            "ERR 83886142 Timeout",
            "OK",
            "S PIN_REPEATED",
            "D x",
            "OK",
            "OK",
        ]
    );

    assert_eq!(d.requests.recv().unwrap()["error"], "Bad PIN");
    assert!(
        d.requests.recv().unwrap().get("error").is_none(),
        "SETERROR is one-shot"
    );
    assert_eq!(d.requests.recv().unwrap()["repeat"], "Again");
    assert!(
        d.requests.recv().unwrap().get("repeat").is_none(),
        "SETREPEAT is one-shot"
    );
}

#[test]
fn confirm_and_message() {
    let d = fake_dialog(vec![
        line(r#"{"result":"ok"}"#),
        line(r#"{"result":"notok"}"#),
        line(r#"{"result":"cancel"}"#),
        line(r#"{"result":"cancel"}"#),
        line(r#"{"result":"timeout"}"#),
    ]);
    let lines = run(
        &d,
        "CONFIRM\nCONFIRM\nCONFIRM\nCONFIRM --one-button\nMESSAGE\n",
    );
    assert_eq!(
        lines,
        [
            "OK",
            "ERR 83886194 Not confirmed",
            "ERR 83886179 Operation cancelled",
            "OK",
            "OK",
        ]
    );
    for _ in 0..3 {
        assert_eq!(d.requests.recv().unwrap()["type"], "confirm");
    }
    assert_eq!(d.requests.recv().unwrap()["type"], "message");
    assert_eq!(d.requests.recv().unwrap()["type"], "message");
}

#[test]
fn dialog_errors_map_to_assuan_errors() {
    let d = fake_dialog(vec![
        line(r#"{"result":"busy"}"#),
        line(r#"{"result":"error","message":"unsupported request"}"#),
        line(r#"{"result":"surprise"}"#),
        line("not json"),
    ]);
    let lines = run(&d, "GETPIN\nGETPIN\nCONFIRM\nGETPIN\n");
    assert_eq!(
        lines,
        [
            "ERR 83886166 Another pinentry dialog is open",
            "ERR 83886166 unsupported request",
            "ERR 83886166 Dialog error",
            "ERR 83886179 Operation cancelled",
        ]
    );
}

#[test]
fn dialog_messages_cannot_inject_assuan_lines() {
    let d = fake_dialog(vec![line(
        r#"{"result":"error","message":"x\nD 1234\r\nOK"}"#,
    )]);
    let lines = run(&d, "GETPIN\n");
    assert_eq!(lines, ["ERR 83886166 x D 1234  OK"]);
}

#[test]
fn oversized_or_missing_replies_are_cancelled() {
    let d = fake_dialog(vec![
        Answer::Raw(vec![b'x'; 70 * 1024]),
        Answer::Raw(vec![]),
    ]);
    let lines = run(&d, "GETPIN\nGETPIN\n");
    assert_eq!(
        lines,
        [
            "ERR 83886179 Operation cancelled",
            "ERR 83886179 Operation cancelled",
        ]
    );
}

#[test]
fn a_hung_dialog_times_out() {
    let d = fake_dialog(vec![Answer::Hang]);
    let started = Instant::now();
    let lines = run(&d, "SETTIMEOUT 1\nGETPIN\n");
    assert_eq!(lines, ["OK", "ERR 83886142 Timeout"]);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn one_connection_per_prompt() {
    let d = fake_dialog(vec![pin_reply("1"), pin_reply("2")]);
    run(&d, "GETPIN\nGETPIN\nBYE\n");
    assert_eq!(d.connections.load(Ordering::SeqCst), 2);

    let d = fake_dialog(vec![]);
    run(&d, "GETINFO version\nBYE\n");
    assert_eq!(d.connections.load(Ordering::SeqCst), 1);
}

#[test]
fn getinfo_and_unknown_commands() {
    let d = fake_dialog(vec![]);
    let lines = run(
        &d,
        "GETINFO flavor\nGETINFO version\nGETINFO pid\nGETINFO nope\nOPTION ttyname=/dev/pts/1\nFROB\n",
    );
    assert_eq!(lines[0], "D omarchy");
    assert_eq!(lines[1], "OK");
    assert_eq!(lines[2], format!("D {}", env!("CARGO_PKG_VERSION")));
    assert!(lines[4].strip_prefix("D ").unwrap().parse::<u32>().is_ok());
    assert_eq!(
        &lines[6..],
        [
            "ERR 83886135 Unknown value for WHAT",
            "OK",
            "ERR 83886355 Unknown IPC command",
        ]
    );
}

#[test]
fn tolerates_case_comments_and_blank_lines() {
    let d = fake_dialog(vec![pin_reply("1")]);
    let lines = run(&d, "# comment\n\nsetprompt PIN:\ngetpin\nbye\n");
    assert_eq!(lines, ["OK", "D 1", "OK", "OK closing connection"]);
    assert_eq!(d.requests.recv().unwrap()["prompt"], "PIN");
}

#[test]
fn reset_clears_the_settings() {
    let d = fake_dialog(vec![pin_reply("1")]);
    run(&d, "SETDESC old\nSETTIMEOUT 9\nRESET\nGETPIN\n");
    let req = d.requests.recv().unwrap();
    assert!(req.get("desc").is_none() && req.get("timeout").is_none());
}

#[test]
fn rejects_bad_timeouts_and_caps_huge_ones() {
    let d = fake_dialog(vec![pin_reply("1")]);
    let lines = run(&d, "SETTIMEOUT abc\nSETTIMEOUT 99999999999\nGETPIN\n");
    assert_eq!(lines[0], "ERR 83886135 invalid timeout");
    assert_eq!(d.requests.recv().unwrap()["timeout"], 86_400);
}

#[test]
fn sanitizes_texts_sent_to_the_dialog() {
    let d = fake_dialog(vec![pin_reply("1")]);
    run(
        &d,
        "SETDESC Key%E2%80%AE of%1B[2J Mallory%0Aline 2\nGETPIN\n",
    );
    assert_eq!(
        d.requests.recv().unwrap()["desc"],
        "Key of[2J Mallory\nline 2"
    );
}

#[test]
fn overlong_lines_are_rejected_and_the_session_goes_on() {
    let d = fake_dialog(vec![]);
    let long = format!("SETDESC {}\n", "x".repeat(2000));
    let lines = run(&d, &format!("{long}GETINFO flavor\n"));
    assert_eq!(lines, ["ERR 83886343 Line too long", "D omarchy", "OK"]);
}

#[test]
fn ends_cleanly_on_eof_without_bye() {
    let d = fake_dialog(vec![]);
    assert_eq!(run(&d, "NOP\n"), ["OK"]);
}

#[test]
fn a_vanished_dialog_counts_as_cancel() {
    // No scripted answers: the fake dialog hangs up after the request.
    let d = fake_dialog(vec![]);
    assert_eq!(run(&d, "GETPIN\n"), ["ERR 83886179 Operation cancelled"]);
}

fn fallback_script(dir: &Path) -> PathBuf {
    let fallback = dir.join("fallback");
    std::fs::write(&fallback, "#!/bin/sh\necho \"OK fallback $*\"\n").unwrap();
    std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755)).unwrap();
    fallback
}

#[test]
fn falls_back_when_the_socket_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(["--display", ":0"])
        .env("PINENTRY_OMARCHY_SOCKET", dir.path().join("missing"))
        .env("PINENTRY_OMARCHY_FALLBACK", fallback_script(dir.path()))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "OK fallback --display :0\n"
    );
}

#[test]
fn falls_back_outside_a_wayland_session() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("PINENTRY_OMARCHY_SOCKET")
        .env("PINENTRY_OMARCHY_FALLBACK", fallback_script(dir.path()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "OK fallback \n");
}

#[test]
fn never_prints_anything_but_protocol_lines() {
    let d = fake_dialog(vec![pin_reply("secret")]);
    let out = run_with(&d.socket, b"GETPIN\n");
    assert!(out.stderr.is_empty());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text.matches("secret").count(),
        1,
        "the PIN appears once, in its D line"
    );
}
