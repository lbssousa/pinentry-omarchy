mod assuan;
mod fallback;
mod request;
mod secret;
mod shell;

use std::env;
use std::io::{self, BufRead, Write};
use std::process;

use assuan::{
    ERR_CANCELED, ERR_INV_VALUE, ERR_NOT_CONFIRMED, ERR_PIN_ENTRY, ERR_TIMEOUT, ERR_UNKNOWN_CMD,
    Writer, split_command,
};
use request::{Kind, State};
use shell::Dialog;

fn main() {
    secret::harden();

    let Some(dialog) = connect_dialog() else {
        let err = fallback::exec();
        eprintln!("pinentry-omarchy: no shell dialog and no fallback pinentry: {err}");
        process::exit(1);
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    if serve(dialog, stdin.lock(), stdout.lock()).is_err() {
        process::exit(1);
    }
}

/// The shell dialog is only an option inside a Wayland session with the
/// plugin listening. PINENTRY_OMARCHY_SOCKET (tests, dev instance) skips
/// the session check. The connection made here serves the first prompt.
fn connect_dialog() -> Option<Dialog> {
    if env::var_os("PINENTRY_OMARCHY_SOCKET").is_none() && env::var_os("WAYLAND_DISPLAY").is_none()
    {
        return None;
    }
    shell::connect().ok().map(Dialog::new)
}

fn serve(mut dialog: Dialog, mut input: impl BufRead, output: impl Write) -> io::Result<()> {
    let mut w = Writer::new(output);
    w.ok(Some(&format!(
        "Pleased to meet you, process {}",
        process::id()
    )))?;

    let mut state = State::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let (cmd, arg) = split_command(&line);
        if cmd.is_empty() || cmd.starts_with('#') {
            continue;
        }
        match state.apply(&cmd, arg) {
            Ok(true) => {
                w.ok(None)?;
                continue;
            }
            Err(msg) => {
                w.err(ERR_INV_VALUE, msg)?;
                continue;
            }
            Ok(false) => {}
        }
        match cmd.as_str() {
            "GETPIN" => getpin(&mut dialog, &mut state, &mut w)?,
            "CONFIRM" => {
                let one_button = String::from_utf8_lossy(arg).contains("--one-button");
                let kind = if one_button {
                    Kind::Message
                } else {
                    Kind::Confirm
                };
                confirm(&mut dialog, &mut state, &mut w, kind)?
            }
            "MESSAGE" => confirm(&mut dialog, &mut state, &mut w, Kind::Message)?,
            "GETINFO" => getinfo(&mut w, &String::from_utf8_lossy(arg))?,
            "RESET" => {
                state = State::default();
                w.ok(None)?
            }
            "BYE" => {
                w.ok(Some("closing connection"))?;
                return Ok(());
            }
            // Accepted and ignored: options, the key's cache id, quality bar
            // and passphrase generation (not offered by this dialog).
            "OPTION" | "SETKEYINFO" | "SETQUALITYBAR" | "SETQUALITYBAR_TT" | "SETGENPIN"
            | "SETGENPIN_TT" | "SETREPEATOK" | "SETCONSTRAINTS" | "CLEARPASSPHRASE" | "NOP"
            | "HELP" => w.ok(None)?,
            _ => w.err(ERR_UNKNOWN_CMD, "Unknown IPC command")?,
        }
    }
}

fn getpin<W: Write>(dialog: &mut Dialog, state: &mut State, w: &mut Writer<W>) -> io::Result<()> {
    let repeat = !state.repeat.is_empty();
    let answer = dialog.ask(&state.request(Kind::GetPin), state.timeout);
    state.after_prompt();
    let response = match answer {
        Ok(r) => r,
        Err(e) => return dialog_failed(w, &e),
    };
    match response.result.as_str() {
        "ok" => {
            if repeat {
                w.status("PIN_REPEATED")?;
            }
            if let Some(pin) = response.pin.as_ref().filter(|p| !p.is_empty()) {
                w.data(pin.as_bytes())?;
            }
            w.ok(None)
        }
        "cancel" => w.err(ERR_CANCELED, "Operation cancelled"),
        "timeout" => w.err(ERR_TIMEOUT, "Timeout"),
        "busy" => w.err(ERR_PIN_ENTRY, "Another pinentry dialog is open"),
        _ => w.err(
            ERR_PIN_ENTRY,
            response.message.as_deref().unwrap_or("Dialog error"),
        ),
    }
}

fn confirm<W: Write>(
    dialog: &mut Dialog,
    state: &mut State,
    w: &mut Writer<W>,
    kind: Kind,
) -> io::Result<()> {
    let message = matches!(kind, Kind::Message);
    let answer = dialog.ask(&state.request(kind), state.timeout);
    state.after_prompt();
    let response = match answer {
        Ok(r) => r,
        Err(e) => return dialog_failed(w, &e),
    };
    if message {
        return w.ok(None);
    }
    match response.result.as_str() {
        "ok" => w.ok(None),
        "notok" => w.err(ERR_NOT_CONFIRMED, "Not confirmed"),
        "cancel" => w.err(ERR_CANCELED, "Operation cancelled"),
        "timeout" => w.err(ERR_TIMEOUT, "Timeout"),
        "busy" => w.err(ERR_PIN_ENTRY, "Another pinentry dialog is open"),
        _ => w.err(
            ERR_PIN_ENTRY,
            response.message.as_deref().unwrap_or("Dialog error"),
        ),
    }
}

/// The shell went away mid-session (restart, crash): there is no answer,
/// so the request counts as cancelled.
fn dialog_failed<W: Write>(w: &mut Writer<W>, err: &io::Error) -> io::Result<()> {
    match err.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => w.err(ERR_TIMEOUT, "Timeout"),
        _ => w.err(ERR_CANCELED, "Operation cancelled"),
    }
}

fn getinfo<W: Write>(w: &mut Writer<W>, what: &str) -> io::Result<()> {
    let value = match what.trim() {
        "version" => env!("CARGO_PKG_VERSION").to_string(),
        "pid" => process::id().to_string(),
        "flavor" => "omarchy".to_string(),
        "ttyinfo" => "- - -".to_string(),
        _ => return w.err(ERR_INV_VALUE, "Unknown value for WHAT"),
    };
    w.data(value.as_bytes())?;
    w.ok(None)
}
