//! Authentication schemes: basic (Cloud token / DC password) and bearer (DC PAT).

use reqwest::header::{HeaderMap,HeaderValue};

#[derive(Debug, Clone)]
pub enum Auth {
    /// Basic auth: Cloud uses email + API token, DC uses username + password.
    Basic { user: String, token: String },
    /// Bearer auth: personal access token on Jira Data Center.
    Bearer { token: String },
}

impl Auth {
    /// Headers applied to every request.
    pub fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        match self {
            Auth::Basic { user, token } => {
                let raw = format!("{user}:{token}");
                // BASE64_STANDARD would require an extra dep; reqwest re-exports none.
                let b64 = base64_encode(raw.as_bytes());
                if let Ok(v) = HeaderValue::from_str(&format!("Basic {b64}")) {
                    h.insert(reqwest::header::AUTHORIZATION, v);
                }
            }
            Auth::Bearer { token } => {
                if let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}")) {
                    h.insert(reqwest::header::AUTHORIZATION, v);
                }
            }
        }
        h
    }
}

/// Minimal standard base64 (RFC 4648) without pulling in a crate.
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}
