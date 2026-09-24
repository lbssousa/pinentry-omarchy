use std::env;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Replaces this process with a stock pinentry, passing our arguments along.
/// Only valid before anything has been written to gpg-agent.
pub fn exec() -> io::Error {
    let program = env::var_os("PINENTRY_OMARCHY_FALLBACK").unwrap_or_else(|| {
        let graphical =
            env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some();
        if graphical {
            "/usr/bin/pinentry-gnome3"
        } else {
            "/usr/bin/pinentry-curses"
        }
        .into()
    });
    Command::new(program).args(env::args_os().skip(1)).exec()
}
