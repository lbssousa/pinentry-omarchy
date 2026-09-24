use serde::Serialize;

use crate::assuan::{percent_decode, strip_mnemonic};

/// Dialog settings accumulated from SET* commands until the next
/// GETPIN / CONFIRM / MESSAGE.
#[derive(Default)]
pub struct State {
    pub title: String,
    pub desc: String,
    pub prompt: String,
    pub error: String,
    pub ok: String,
    pub cancel: String,
    pub notok: String,
    pub repeat: String,
    pub repeat_error: String,
    pub timeout: u64,
}

#[derive(Serialize)]
pub struct Request<'a> {
    v: u32,
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    title: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    desc: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    prompt: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    error: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    ok: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    cancel: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    notok: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    repeat: &'a str,
    #[serde(rename = "repeatError", skip_serializing_if = "str::is_empty")]
    repeat_error: &'a str,
    #[serde(skip_serializing_if = "is_zero")]
    timeout: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

pub enum Kind {
    GetPin,
    Confirm,
    Message,
}

impl State {
    /// Applies a SET* command. Returns false for commands this module
    /// doesn't own.
    pub fn apply(&mut self, cmd: &str, arg: &[u8]) -> Result<bool, &'static str> {
        let text = || percent_decode(arg);
        match cmd {
            "SETTITLE" => self.title = text(),
            "SETDESC" => self.desc = text(),
            "SETPROMPT" => self.prompt = text(),
            "SETERROR" => self.error = text(),
            "SETOK" => self.ok = strip_mnemonic(&text()),
            "SETCANCEL" => self.cancel = strip_mnemonic(&text()),
            "SETNOTOK" => self.notok = strip_mnemonic(&text()),
            "SETREPEAT" => {
                let prompt = text();
                self.repeat = if prompt.is_empty() { "Repeat".into() } else { prompt };
            }
            "SETREPEATERROR" => self.repeat_error = text(),
            "SETTIMEOUT" => {
                self.timeout = text().trim().parse().map_err(|_| "invalid timeout")?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn request(&self, kind: Kind) -> Request<'_> {
        let (kind, getpin) = match kind {
            Kind::GetPin => ("getpin", true),
            Kind::Confirm => ("confirm", false),
            Kind::Message => ("message", false),
        };
        // The prompt usually ends with a colon for terminal pinentries; as a
        // placeholder it reads better without one.
        let prompt = self.prompt.trim().trim_end_matches(':');
        let repeat = self.repeat.trim().trim_end_matches(':');
        Request {
            v: 1,
            kind,
            title: &self.title,
            desc: &self.desc,
            prompt: if getpin { prompt } else { "" },
            error: &self.error,
            ok: &self.ok,
            cancel: &self.cancel,
            notok: &self.notok,
            repeat: if getpin { repeat } else { "" },
            repeat_error: if getpin { &self.repeat_error } else { "" },
            timeout: self.timeout,
        }
    }

    /// SETERROR and SETREPEAT apply to the next prompt only, as in the
    /// upstream pinentry.
    pub fn after_prompt(&mut self) {
        self.error.clear();
        self.repeat.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_getpin_request() {
        let mut s = State::default();
        assert!(s.apply("SETDESC", b"Unlock%0Acard").unwrap());
        assert!(s.apply("SETPROMPT", b"PIN:").unwrap());
        assert!(s.apply("SETOK", b"_OK").unwrap());
        assert!(!s.apply("GETPIN", b"").unwrap());
        let json = serde_json::to_string(&s.request(Kind::GetPin)).unwrap();
        assert_eq!(json, r#"{"v":1,"type":"getpin","desc":"Unlock\ncard","prompt":"PIN","ok":"OK"}"#);
    }

    #[test]
    fn error_is_one_shot() {
        let mut s = State::default();
        s.apply("SETERROR", b"Bad PIN").unwrap();
        s.apply("SETREPEAT", b"").unwrap();
        assert_eq!(s.request(Kind::GetPin).repeat, "Repeat");
        s.after_prompt();
        assert!(s.error.is_empty() && s.repeat.is_empty());
    }

    #[test]
    fn rejects_bad_timeout() {
        assert!(State::default().apply("SETTIMEOUT", b"soon").is_err());
    }
}
