#![forbid(unsafe_code)]

use std::env;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::process;

use pinentry_omarchy::shell::{self, Dialog};
use pinentry_omarchy::{fallback, secret, session};

fn main() {
    secret::harden();

    let Some(dialog) = connect_dialog() else {
        let err = fallback::exec();
        eprintln!("pinentry-omarchy: no shell dialog and no fallback pinentry: {err}");
        process::exit(1);
    };

    // Responses go straight to fd 1 through an unbuffered File: std's
    // stdout buffer would keep the last "D <pin>" line around unwiped.
    let stdout = io::stdout().as_fd().try_clone_to_owned().map(File::from);
    let Ok(stdout) = stdout else {
        process::exit(1);
    };
    if session::serve(dialog, io::stdin().lock(), stdout).is_err() {
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
