use serde::Serialize;

use crate::assuan::{percent_decode, sanitize_display, strip_mnemonic};

/// Protocol version spoken with the plugin; a plugin from another version
/// rejects the request instead of misreading the PIN.
pub const PROTOCOL_VERSION: u32 = 2;

/// Longest SETTIMEOUT honoured (one day). Keeps the plugin's millisecond
/// timer and our read deadline in range.
pub const MAX_TIMEOUT_SECS: u64 = 86_400;

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
        let text = || sanitize_display(&percent_decode(arg));
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
                self.repeat = if prompt.is_empty() {
                    "Repeat".into()
                } else {
                    prompt
                };
            }
            "SETREPEATERROR" => self.repeat_error = text(),
            "SETTIMEOUT" => {
                let secs: u64 = percent_decode(arg)
                    .trim()
                    .parse()
                    .map_err(|_| "invalid timeout")?;
                self.timeout = secs.min(MAX_TIMEOUT_SECS);
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
            v: PROTOCOL_VERSION,
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

    fn json(state: &State, kind: Kind) -> serde_json::Value {
        serde_json::to_value(state.request(kind)).unwrap()
    }

    #[test]
    fn builds_getpin_request() {
        let mut s = State::default();
        assert!(s.apply("SETDESC", b"Unlock%0Acard").unwrap());
        assert!(s.apply("SETPROMPT", b"PIN:").unwrap());
        assert!(s.apply("SETOK", b"_OK").unwrap());
        assert!(!s.apply("GETPIN", b"").unwrap());
        let json = serde_json::to_string(&s.request(Kind::GetPin)).unwrap();
        assert_eq!(
            json,
            r#"{"v":2,"type":"getpin","desc":"Unlock\ncard","prompt":"PIN","ok":"OK"}"#
        );
    }

    #[test]
    fn maps_every_set_command() {
        let mut s = State::default();
        for (cmd, arg) in [
            ("SETTITLE", "Title"),
            ("SETDESC", "Desc"),
            ("SETPROMPT", "Prompt:"),
            ("SETERROR", "Error"),
            ("SETOK", "_Yes"),
            ("SETCANCEL", "_Abort"),
            ("SETNOTOK", "_No"),
            ("SETREPEAT", "Again:"),
            ("SETREPEATERROR", "Mismatch"),
            ("SETTIMEOUT", "30"),
        ] {
            assert!(s.apply(cmd, arg.as_bytes()).unwrap(), "{cmd}");
        }
        assert_eq!(
            json(&s, Kind::GetPin),
            serde_json::json!({
                "v": 2, "type": "getpin", "title": "Title", "desc": "Desc",
                "prompt": "Prompt", "error": "Error", "ok": "Yes", "cancel": "Abort",
                "notok": "No", "repeat": "Again", "repeatError": "Mismatch", "timeout": 30,
            })
        );
    }

    #[test]
    fn confirm_and_message_omit_pin_fields() {
        let mut s = State::default();
        s.apply("SETPROMPT", b"PIN:").unwrap();
        s.apply("SETREPEAT", b"").unwrap();
        s.apply("SETREPEATERROR", b"x").unwrap();
        s.apply("SETDESC", b"Sure?").unwrap();
        for kind in [Kind::Confirm, Kind::Message] {
            let v = json(&s, kind);
            assert!(v.get("prompt").is_none() && v.get("repeat").is_none());
            assert!(v.get("repeatError").is_none());
            assert_eq!(v["desc"], "Sure?");
        }
        assert_eq!(json(&s, Kind::Confirm)["type"], "confirm");
        assert_eq!(json(&s, Kind::Message)["type"], "message");
    }

    #[test]
    fn error_and_repeat_are_one_shot() {
        let mut s = State::default();
        s.apply("SETERROR", b"Bad PIN").unwrap();
        s.apply("SETREPEAT", b"").unwrap();
        s.apply("SETDESC", b"stays").unwrap();
        assert_eq!(s.request(Kind::GetPin).repeat, "Repeat");
        s.after_prompt();
        assert!(s.error.is_empty() && s.repeat.is_empty());
        assert_eq!(s.desc, "stays");
    }

    #[test]
    fn caps_and_validates_timeout() {
        let mut s = State::default();
        s.apply("SETTIMEOUT", b" 45 ").unwrap();
        assert_eq!(s.timeout, 45);
        s.apply("SETTIMEOUT", b"99999999999").unwrap();
        assert_eq!(s.timeout, MAX_TIMEOUT_SECS);
        for bad in [&b"soon"[..], b"-1", b"", b"1.5", b"99999999999999999999999"] {
            assert!(s.apply("SETTIMEOUT", bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            s.timeout, MAX_TIMEOUT_SECS,
            "a rejected value keeps the old one"
        );
    }

    #[test]
    fn sanitizes_texts() {
        let mut s = State::default();
        s.apply(
            "SETDESC",
            "Key of Mallory %E2%80%AEgpj.exe%1B[2J".as_bytes(),
        )
        .unwrap();
        s.apply("SETTITLE", b"a%0Db").unwrap();
        s.apply("SETOK", b"%E2%81%A6_Sign").unwrap();
        assert_eq!(s.desc, "Key of Mallory gpj.exe[2J");
        assert_eq!(s.title, "ab");
        assert_eq!(s.ok, "Sign");
    }
}
