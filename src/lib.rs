//! pinentry-omarchy's protocol code, split from the binary so the fuzz
//! targets (fuzz/) can drive it.

#![forbid(unsafe_code)]

pub mod assuan;
pub mod fallback;
pub mod request;
pub mod secret;
pub mod session;
pub mod shell;
