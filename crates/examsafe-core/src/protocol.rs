//! Wire format between the (unprivileged) app and the (elevated) helper.
//!
//! The request travels on the helper's command line, hex-encoded: the command line is fixed at
//! launch time and shown by UAC, so no other process can swap the request afterwards. The
//! response is written to a file whose name the helper validates.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const PROTOCOL_VERSION: u32 = 1;
pub const RESPONSE_FILE_PREFIX: &str = "examsafe-helper-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperAction {
    /// Handshake only: proves the helper started and reports whether it is elevated.
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperRequest {
    pub protocol: u32,
    pub id: String,
    pub action: HelperAction,
    pub response_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperResponse {
    pub protocol: u32,
    pub id: String,
    pub ok: bool,
    pub elevated: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("the request is not valid hex")]
    BadEncoding,
    #[error("the request is not valid JSON: {0}")]
    BadJson(String),
    #[error("protocol version {found} is not supported (expected {expected})")]
    Version { found: u32, expected: u32 },
    #[error("response path is not allowed: {0}")]
    BadResponsePath(String),
}

pub fn encode_request(request: &HelperRequest) -> Result<String, ProtocolError> {
    let json =
        serde_json::to_vec(request).map_err(|error| ProtocolError::BadJson(error.to_string()))?;
    Ok(to_hex(&json))
}

pub fn decode_request(encoded: &str) -> Result<HelperRequest, ProtocolError> {
    let bytes = from_hex(encoded).ok_or(ProtocolError::BadEncoding)?;
    let request: HelperRequest = serde_json::from_slice(&bytes)
        .map_err(|error| ProtocolError::BadJson(error.to_string()))?;
    if request.protocol != PROTOCOL_VERSION {
        return Err(ProtocolError::Version {
            found: request.protocol,
            expected: PROTOCOL_VERSION,
        });
    }
    validate_response_path(&request.response_path)?;
    Ok(request)
}

/// The elevated helper writes wherever this path points, so only accept our own file names.
pub fn validate_response_path(path: &str) -> Result<(), ProtocolError> {
    let reject = || Err(ProtocolError::BadResponsePath(path.to_owned()));
    let parsed = Path::new(path);
    if !parsed.is_absolute() || parsed.components().any(|c| c == Component::ParentDir) {
        return reject();
    }
    match parsed.file_name().and_then(|name| name.to_str()) {
        Some(name) if name.starts_with(RESPONSE_FILE_PREFIX) && name.ends_with(".json") => Ok(()),
        _ => reject(),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_path() -> String {
        if cfg!(windows) {
            r"C:\Users\x\AppData\Local\Temp\examsafe-helper-1.json".to_owned()
        } else {
            "/tmp/examsafe-helper-1.json".to_owned()
        }
    }

    fn sample_request() -> HelperRequest {
        HelperRequest {
            protocol: PROTOCOL_VERSION,
            id: "abc".into(),
            action: HelperAction::Ping,
            response_path: sample_path(),
        }
    }

    #[test]
    fn request_round_trips() {
        let encoded = encode_request(&sample_request()).unwrap();
        assert!(encoded.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(decode_request(&encoded).unwrap(), sample_request());
    }

    #[test]
    fn rejects_bad_hex() {
        assert_eq!(decode_request("zz"), Err(ProtocolError::BadEncoding));
        assert_eq!(decode_request("abc"), Err(ProtocolError::BadEncoding));
    }

    #[test]
    fn rejects_other_protocol_versions() {
        let mut request = sample_request();
        request.protocol = 2;
        let encoded = encode_request(&request).unwrap();
        assert!(matches!(
            decode_request(&encoded),
            Err(ProtocolError::Version { found: 2, .. })
        ));
    }

    #[test]
    fn response_path_must_be_our_own_file() {
        assert!(validate_response_path(&sample_path()).is_ok());
        assert!(validate_response_path("examsafe-helper-1.json").is_err());
        let prefix = if cfg!(windows) {
            r"C:\Windows\"
        } else {
            "/etc/"
        };
        assert!(validate_response_path(&format!("{prefix}system.ini")).is_err());
        assert!(validate_response_path(&format!("{prefix}examsafe-helper-1.txt")).is_err());
        let traversal = if cfg!(windows) {
            r"C:\Temp\..\Windows\examsafe-helper-1.json"
        } else {
            "/tmp/../etc/examsafe-helper-1.json"
        };
        assert!(validate_response_path(traversal).is_err());
    }

    #[test]
    fn hex_encoding_is_lowercase_and_reversible() {
        assert_eq!(to_hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(from_hex("00ABff"), Some(vec![0x00, 0xab, 0xff]));
    }
}
