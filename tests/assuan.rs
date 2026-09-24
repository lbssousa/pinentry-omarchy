use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

const BIN: &str = env!("CARGO_BIN_EXE_pinentry-omarchy");

/// Fake shell plugin: answers each request line with the next scripted
/// response and reports the requests it saw. Connections that send nothing
/// (the availability probe) are skipped.
fn fake_dialog(path: &Path, responses: Vec<&'static str>) -> mpsc::Receiver<serde_json::Value> {
    let listener = UnixListener::bind(path).unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut responses = responses.into_iter();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).unwrap() == 0 {
                continue;
            }
            tx.send(serde_json::from_str(&line).unwrap()).unwrap();
            let Some(response) = responses.next() else { return };
            writeln!(stream, "{response}").unwrap();
        }
    });
    rx
}

fn run(socket: &Path, script: &str) -> String {
    let mut child = Command::new(BIN)
        .env("PINENTRY_OMARCHY_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn getpin_returns_escaped_pin() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("s");
    let requests = fake_dialog(&sock, vec![r#"{"result":"ok","pin":"ab%c"}"#]);

    let out = run(&sock, "SETDESC Hello%0AWorld\nSETPROMPT PIN:\nSETOK _Unlock\nGETPIN\nBYE\n");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("OK Pleased to meet you"));
    assert_eq!(&lines[1..], ["OK", "OK", "OK", "D ab%25c", "OK", "OK closing connection"]);

    let req = requests.recv().unwrap();
    assert_eq!(req["type"], "getpin");
    assert_eq!(req["desc"], "Hello\nWorld");
    assert_eq!(req["prompt"], "PIN");
    assert_eq!(req["ok"], "Unlock");
}

#[test]
fn cancel_timeout_and_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("s");
    let requests = fake_dialog(
        &sock,
        vec![
            r#"{"result":"cancel"}"#,
            r#"{"result":"timeout"}"#,
            r#"{"result":"ok","pin":"x"}"#,
            r#"{"result":"ok"}"#,
        ],
    );

    let out = run(
        &sock,
        "SETERROR Bad PIN\nGETPIN\nGETPIN\nSETREPEAT Again:\nGETPIN\nGETPIN\n",
    );
    let lines: Vec<&str> = out.lines().skip(1).collect();
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

    assert_eq!(requests.recv().unwrap()["error"], "Bad PIN");
    assert!(requests.recv().unwrap().get("error").is_none(), "SETERROR is one-shot");
    assert_eq!(requests.recv().unwrap()["repeat"], "Again");
    assert!(requests.recv().unwrap().get("repeat").is_none(), "SETREPEAT is one-shot");
}

#[test]
fn confirm_and_message() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("s");
    let requests = fake_dialog(
        &sock,
        vec![
            r#"{"result":"ok"}"#,
            r#"{"result":"notok"}"#,
            r#"{"result":"cancel"}"#,
            r#"{"result":"cancel"}"#,
        ],
    );

    let out = run(&sock, "CONFIRM\nCONFIRM\nCONFIRM\nCONFIRM --one-button\n");
    let lines: Vec<&str> = out.lines().skip(1).collect();
    assert_eq!(
        lines,
        [
            "OK",
            "ERR 83886194 Not confirmed",
            "ERR 83886179 Operation cancelled",
            "OK",
        ]
    );
    for _ in 0..3 {
        assert_eq!(requests.recv().unwrap()["type"], "confirm");
    }
    assert_eq!(requests.recv().unwrap()["type"], "message");
}

#[test]
fn getinfo_and_unknown_commands() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("s");
    let _requests = fake_dialog(&sock, vec![]);

    let out = run(&sock, "GETINFO flavor\nOPTION ttyname=/dev/pts/1\nFROB\n");
    let lines: Vec<&str> = out.lines().skip(1).collect();
    assert_eq!(lines, ["D omarchy", "OK", "OK", "ERR 83886355 Unknown IPC command"]);
}

#[test]
fn dialog_gone_counts_as_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("s");
    // No scripted responses: the fake server hangs up after the request.
    let _requests = fake_dialog(&sock, vec![]);

    let out = run(&sock, "GETPIN\n");
    assert_eq!(out.lines().nth(1), Some("ERR 83886179 Operation cancelled"));
}

#[test]
fn falls_back_without_a_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let fallback = dir.path().join("fallback");
    std::fs::write(&fallback, "#!/bin/sh\necho \"OK fallback $*\"\n").unwrap();
    std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = Command::new(BIN)
        .arg("--display")
        .arg(":0")
        .env("PINENTRY_OMARCHY_SOCKET", dir.path().join("missing"))
        .env("PINENTRY_OMARCHY_FALLBACK", &fallback)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "OK fallback --display :0\n");
}
