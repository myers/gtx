//! Curl-style HTTP transcripts for debugging.
//!
//! Set [`set_config`] once at startup. Every progenitor-typed request and
//! every raw [`Gitea::request`](crate::Gitea::request) call will then emit a
//! `>`-prefixed request transcript and `<`-prefixed response transcript on
//! stderr. Authorization tokens are masked unless `show_secrets` is set.

use std::io::{self, Write};
use std::sync::OnceLock;

use progenitor_client::{ClientHooks, ClientInfo, Error, OperationInfo};

/// Verbosity level for HTTP transcripts.
#[derive(Debug, Clone, Copy, Default)]
pub struct VerboseConfig {
    /// 0 = off, 1 = headers only, 2+ = headers + bodies.
    pub level: u8,
    /// When false, redact `Authorization` and `Cookie` header values.
    pub show_secrets: bool,
}

impl VerboseConfig {
    pub fn is_on(&self) -> bool {
        self.level > 0
    }
    pub fn show_body(&self) -> bool {
        self.level >= 2
    }
}

static CONFIG: OnceLock<VerboseConfig> = OnceLock::new();

/// Install the verbose configuration. First call wins (subsequent calls are
/// ignored — verbose state is process-global).
pub fn set_config(cfg: VerboseConfig) {
    let _ = CONFIG.set(cfg);
}

/// Current verbose configuration, or `None` if not set.
pub fn config() -> Option<&'static VerboseConfig> {
    CONFIG.get()
}

static AUTH_HEADERS: OnceLock<reqwest::header::HeaderMap> = OnceLock::new();

/// Install the auth header map that will be injected onto every outgoing
/// request via the `pre` hook. Called once by [`Gitea::new`](crate::Gitea).
pub fn set_auth_headers(headers: reqwest::header::HeaderMap) {
    let _ = AUTH_HEADERS.set(headers);
}

/// Apply the configured auth headers to a request that doesn't already have
/// them. Idempotent — won't overwrite headers the caller already set.
pub fn apply_auth_headers(request: &mut reqwest::Request) {
    let Some(auth) = AUTH_HEADERS.get() else {
        return;
    };
    let headers = request.headers_mut();
    for (name, value) in auth.iter() {
        if !headers.contains_key(name) {
            headers.insert(name.clone(), value.clone());
        }
    }
}

/// Mask credential-bearing header values, leaving the last 4 chars visible
/// when the value is long enough (so users can disambiguate which token).
pub fn mask_header_value(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let is_secret = lower == "authorization" || lower == "cookie" || lower.ends_with("-token");
    if !is_secret {
        return value.to_string();
    }

    // Authorization tends to be `token abcdef...` or `Bearer abcdef...`.
    if let Some(rest) = value.strip_prefix("token ") {
        return format!("token {}", mask_secret(rest));
    }
    if let Some(rest) = value.strip_prefix("Bearer ") {
        return format!("Bearer {}", mask_secret(rest));
    }
    mask_secret(value)
}

fn mask_secret(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() > 8 {
        format!("***{}", &trimmed[trimmed.len() - 4..])
    } else {
        "***".to_string()
    }
}

/// Print a `>`-prefixed request transcript on stderr.
pub fn print_request(request: &reqwest::Request, op_id: Option<&str>, cfg: &VerboseConfig) {
    let mut out = io::stderr().lock();
    let url = request.url();
    let path_q = match url.query() {
        Some(q) => format!("{}?{}", url.path(), q),
        None => url.path().to_string(),
    };
    if let Some(id) = op_id {
        let _ = writeln!(out, "* operation: {id}");
    }
    let _ = writeln!(out, "> {} {} HTTP/1.1", request.method(), path_q);
    if let Some(host) = url.host_str() {
        let _ = writeln!(out, "> Host: {host}");
    }
    for (name, value) in request.headers() {
        let raw = value.to_str().unwrap_or("<binary>");
        let shown = if cfg.show_secrets {
            raw.to_string()
        } else {
            mask_header_value(name.as_str(), raw)
        };
        let _ = writeln!(out, "> {}: {}", name, shown);
    }
    let _ = writeln!(out, ">");
    if cfg.show_body() {
        if let Some(body) = request.body().and_then(|b| b.as_bytes()) {
            match std::str::from_utf8(body) {
                Ok(s) => {
                    for line in s.lines() {
                        let _ = writeln!(out, "> {line}");
                    }
                    let _ = writeln!(out, ">");
                }
                Err(_) => {
                    let _ = writeln!(out, "> [{} bytes of binary body]", body.len());
                    let _ = writeln!(out, ">");
                }
            }
        }
    }
}

/// Print a `<`-prefixed response transcript on stderr (status + headers only).
/// The response body must be logged separately by the caller, since reading
/// it consumes the response.
pub fn print_response_head(response: &reqwest::Response, cfg: &VerboseConfig) {
    let mut out = io::stderr().lock();
    let _ = writeln!(
        out,
        "< HTTP/1.1 {} {}",
        response.status().as_u16(),
        response.status().canonical_reason().unwrap_or(""),
    );
    for (name, value) in response.headers() {
        let raw = value.to_str().unwrap_or("<binary>");
        let shown = if cfg.show_secrets {
            raw.to_string()
        } else {
            mask_header_value(name.as_str(), raw)
        };
        let _ = writeln!(out, "< {}: {}", name, shown);
    }
    let _ = writeln!(out, "<");
}

/// Print a body block on stderr in `<`-prefixed style. Use after the caller
/// has consumed the response body.
pub fn print_response_body(body: &str) {
    let mut out = io::stderr().lock();
    for line in body.lines() {
        let _ = writeln!(out, "< {line}");
    }
    let _ = writeln!(out, "<");
}

/// Override progenitor's default no-op hooks for the generated `Client`.
/// Auto-ref specialization causes this impl (on `Client`) to win over the
/// blanket `impl ClientHooks for &Client` that progenitor emits.
impl ClientHooks<()> for crate::Client {
    async fn pre<E>(
        &self,
        request: &mut reqwest::Request,
        info: &OperationInfo,
    ) -> Result<(), Error<E>> {
        apply_auth_headers(request);
        if let Some(cfg) = config()
            && cfg.is_on()
        {
            print_request(request, Some(info.operation_id), cfg);
        }
        Ok(())
    }

    async fn exec(
        &self,
        request: reqwest::Request,
        _info: &OperationInfo,
    ) -> reqwest::Result<reqwest::Response> {
        let response = self.client().execute(request).await?;
        if let Some(cfg) = config()
            && cfg.is_on()
        {
            print_response_head(&response, cfg);
        }
        if response.status().is_success() {
            return Ok(response);
        }
        capture_error_body(response).await
    }
}

/// A failed response's body, read by the `exec` hook so that the sync
/// `From<progenitor_client::Error>` for [`crate::GiteaError`] can report the
/// server's reason. Rides on the response as an extension.
#[derive(Clone)]
pub(crate) struct ErrorBody {
    pub url: String,
    pub body: String,
}

/// Read a failed response's body and rebuild the response around it, with
/// the text (and the request URL, which a rebuilt response loses) attached
/// as an [`ErrorBody`] extension.
async fn capture_error_body(response: reqwest::Response) -> reqwest::Result<reqwest::Response> {
    let url = response.url().to_string();
    let mut builder = http::Response::builder()
        .status(response.status())
        .version(response.version());
    if let Some(headers) = builder.headers_mut() {
        *headers = response.headers().clone();
    }
    let bytes = response.bytes().await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let mut rebuilt: reqwest::Response = builder
        .body(bytes)
        .expect("status, version and headers came from a valid response")
        .into();
    rebuilt.extensions_mut().insert(ErrorBody { url, body });
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_token_header() {
        assert_eq!(
            mask_header_value("Authorization", "token aafe123456789a263"),
            "token ***a263",
        );
        assert_eq!(
            mask_header_value("authorization", "Bearer xyz12345abcdEF98"),
            "Bearer ***EF98",
        );
    }

    #[test]
    fn mask_short_token() {
        assert_eq!(
            mask_header_value("Authorization", "token short"),
            "token ***",
        );
    }

    #[test]
    fn mask_cookie() {
        assert_eq!(
            mask_header_value("Cookie", "session=verysecretcookievalue"),
            "***alue",
        );
    }

    #[test]
    fn mask_x_token_header() {
        assert_eq!(
            mask_header_value("X-Csrf-Token", "abcdef1234567890"),
            "***7890",
        );
    }

    #[test]
    fn mask_skips_non_secret_headers() {
        assert_eq!(mask_header_value("Accept", "application/json"), "application/json");
        assert_eq!(mask_header_value("Content-Type", "application/json"), "application/json");
    }

    #[test]
    fn config_levels() {
        let off = VerboseConfig { level: 0, show_secrets: false };
        assert!(!off.is_on());
        assert!(!off.show_body());

        let v = VerboseConfig { level: 1, show_secrets: false };
        assert!(v.is_on());
        assert!(!v.show_body());

        let vv = VerboseConfig { level: 2, show_secrets: false };
        assert!(vv.is_on());
        assert!(vv.show_body());
    }
}
