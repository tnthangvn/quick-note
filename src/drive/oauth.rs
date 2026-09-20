//! Google OAuth 2.0 for installed apps: loopback redirect + PKCE (RFC 7636).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::config::DriveConfig;
use crate::storage;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
const TOKEN_FILE: &str = "drive_token.json";
/// Refresh slightly early so a token never expires mid-request.
const EXPIRY_MARGIN_SECS: i64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub refresh_token: String,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
}

pub struct Session {
    cfg: DriveConfig,
    refresh_token: String,
    access_token: String,
    expires_at: i64,
}

impl Session {
    /// Restores a session from the saved refresh token, if it matches the configured scope.
    pub fn restore(cfg: &DriveConfig, dir: &Path) -> Result<Option<Self>> {
        let token_path = dir.join(TOKEN_FILE);
        let Some(stored) = storage::read_json::<StoredToken>(&token_path)? else {
            return Ok(None);
        };
        if stored.scope != cfg.scope() {
            return Ok(None);
        }
        Ok(Some(Self {
            cfg: cfg.clone(),
            refresh_token: stored.refresh_token,
            access_token: String::new(),
            expires_at: 0,
        }))
    }

    /// Runs the interactive browser login and persists the refresh token.
    pub fn login(cfg: &DriveConfig, dir: &Path, http: &reqwest::blocking::Client) -> Result<Self> {
        if !cfg.is_configured() {
            bail!("chưa nhập client_id / client_secret trong Cài đặt");
        }
        let listener = TcpListener::bind("127.0.0.1:0").context("không mở được cổng loopback")?;
        let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        let pkce = Pkce::generate();
        let state = Uuid::new_v4().simple().to_string();

        let auth_url = build_auth_url(cfg, &redirect_uri, &pkce.challenge, &state)?;
        webbrowser::open(auth_url.as_str()).context("không mở được trình duyệt")?;

        let code = wait_for_code(&listener, &state)?;
        let token = exchange(
            http,
            &[
                ("code", code.as_str()),
                ("client_id", cfg.client_id.trim()),
                ("client_secret", cfg.client_secret.trim()),
                ("redirect_uri", redirect_uri.as_str()),
                ("grant_type", "authorization_code"),
                ("code_verifier", pkce.verifier.as_str()),
            ],
        )?;
        let refresh_token = token
            .refresh_token
            .ok_or_else(|| anyhow!("Google không trả refresh_token"))?;

        let token_path = dir.join(TOKEN_FILE);
        let stored = StoredToken {
            refresh_token: refresh_token.clone(),
            scope: cfg.scope().into(),
        };
        storage::write_json(&token_path, &stored, true)?;

        Ok(Self {
            cfg: cfg.clone(),
            refresh_token,
            access_token: token.access_token,
            expires_at: crate::model::now_ts() + token.expires_in - EXPIRY_MARGIN_SECS,
        })
    }

    /// Returns a valid access token, refreshing it when needed.
    pub fn access_token(&mut self, http: &reqwest::blocking::Client) -> Result<String> {
        if self.access_token.is_empty() || crate::model::now_ts() >= self.expires_at {
            let token = exchange(
                http,
                &[
                    ("refresh_token", self.refresh_token.as_str()),
                    ("client_id", self.cfg.client_id.trim()),
                    ("client_secret", self.cfg.client_secret.trim()),
                    ("grant_type", "refresh_token"),
                ],
            )?;
            self.access_token = token.access_token;
            self.expires_at = crate::model::now_ts() + token.expires_in - EXPIRY_MARGIN_SECS;
        }
        Ok(self.access_token.clone())
    }
}

/// Forgets the local token (does not revoke it at Google).
pub fn forget_token(dir: &Path) -> Result<()> {
    let path = dir.join(TOKEN_FILE);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn generate() -> Self {
        // 64 hex chars from two v4 UUIDs (OS CSPRNG) — within RFC 7636's 43..128 range.
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let challenge = pkce_challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn build_auth_url(
    cfg: &DriveConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<Url> {
    let mut url = Url::parse(AUTH_URL)?;
    url.query_pairs_mut()
        .append_pair("client_id", cfg.client_id.trim())
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", cfg.scope())
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    Ok(url)
}

fn exchange(http: &reqwest::blocking::Client, params: &[(&str, &str)]) -> Result<TokenResponse> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    let resp = http
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .context("gọi token endpoint")?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        if is_invalid_grant(&text) {
            return Err(InvalidGrant.into());
        }
        bail!("đổi token thất bại ({status}): {}", google_error(&text));
    }
    Ok(serde_json::from_str(&text)?)
}

/// Refresh token expired or revoked — the user has to log in again.
#[derive(Debug)]
pub struct InvalidGrant;

impl std::fmt::Display for InvalidGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("phiên Google Drive đã hết hạn hoặc bị thu hồi — hãy kết nối lại")
    }
}

impl std::error::Error for InvalidGrant {}

fn is_invalid_grant(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .is_ok_and(|v| v.get("error").and_then(|e| e.as_str()) == Some("invalid_grant"))
}

/// Extracts a short error description without echoing tokens back.
pub(super) fn google_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let err = v.get("error_description").or_else(|| v.get("error"))?;
            Some(
                err.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| err.to_string()),
            )
        })
        .unwrap_or_else(|| "không rõ lỗi".into())
}

fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Browsers may open extra connections (favicon, preconnect); skip those.
                if let Some(result) = handle_redirect(stream, expected_state)? {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    bail!("hết thời gian chờ đăng nhập Google");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn handle_redirect(mut stream: TcpStream, expected_state: &str) -> Result<Option<Result<String>>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request_line = String::new();
    BufReader::new(&stream).read_line(&mut request_line)?;

    let Some(outcome) = parse_redirect(&request_line, expected_state) else {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return Ok(None);
    };
    let msg = match &outcome {
        Ok(_) => "Đăng nhập thành công. Có thể đóng tab này và quay lại Quick Note.",
        Err(_) => "Đăng nhập thất bại. Quay lại Quick Note để xem lỗi.",
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Quick Note</title><p style=\"font:16px sans-serif\">{msg}</p>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes());
    Ok(Some(outcome))
}

/// Parses `GET /?code=..&state=.. HTTP/1.1`. `None` = not an OAuth redirect.
fn parse_redirect(request_line: &str, expected_state: &str) -> Option<Result<String>> {
    let target = request_line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    if code.is_none() && error.is_none() {
        return None;
    }
    if state.as_deref() != Some(expected_state) {
        return Some(Err(anyhow!("state OAuth không khớp — bỏ qua phản hồi")));
    }
    if let Some(err) = error {
        return Some(Err(anyhow!("Google từ chối: {err}")));
    }
    code.map(Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // Appendix B of RFC 7636.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pkce_verifier_length_in_range() {
        let p = Pkce::generate();
        assert!((43..=128).contains(&p.verifier.len()));
    }

    #[test]
    fn parse_redirect_accepts_matching_state() {
        let r = parse_redirect("GET /?state=s1&code=4%2Fabc HTTP/1.1", "s1").unwrap();
        assert_eq!(r.unwrap(), "4/abc");
    }

    #[test]
    fn parse_redirect_rejects_wrong_state() {
        let r = parse_redirect("GET /?state=evil&code=x HTTP/1.1", "s1").unwrap();
        assert!(r.is_err());
    }

    #[test]
    fn parse_redirect_reports_denied() {
        let r = parse_redirect("GET /?state=s1&error=access_denied HTTP/1.1", "s1").unwrap();
        assert!(r.unwrap_err().to_string().contains("access_denied"));
    }

    #[test]
    fn parse_redirect_ignores_favicon() {
        assert!(parse_redirect("GET /favicon.ico HTTP/1.1", "s1").is_none());
    }

    #[test]
    fn auth_url_contains_pkce_and_offline() {
        let cfg = DriveConfig {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            ..Default::default()
        };
        let url = build_auth_url(&cfg, "http://127.0.0.1:1234", "chal", "st").unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:1234");
        assert!(
            !url.as_str().contains("sec"),
            "secret must never be in auth URL"
        );
    }

    #[test]
    fn detects_invalid_grant() {
        assert!(is_invalid_grant(
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#
        ));
        assert!(!is_invalid_grant(r#"{"error":"invalid_client"}"#));
    }

    #[test]
    fn google_error_extracts_description() {
        assert_eq!(
            google_error(r#"{"error":"invalid_grant","error_description":"Bad"}"#),
            "Bad"
        );
        assert_eq!(google_error("oops"), "không rõ lỗi");
    }
}
