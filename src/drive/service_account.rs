//! Service-account auth: sign a JWT with the key file and swap it for an access
//! token. No browser, no 7-day refresh-token expiry.
//!
//! Files a service account creates are owned by it, and it has no storage quota
//! of its own — so the target folder must live in a **Shared Drive**.

use std::path::Path;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

use crate::model::now_ts;

const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const TOKEN_LIFETIME: i64 = 3600;
/// Refresh slightly early so a token never expires mid-request.
const EXPIRY_MARGIN_SECS: i64 = 60;

#[derive(Debug, Deserialize)]
struct KeyFile {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".into()
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

pub struct ServiceAccount {
    email: String,
    /// Tài khoản được đóng vai (claim `sub`), nếu bật uỷ quyền toàn miền.
    subject: Option<String>,
    /// PKCS#8 DER, as extracted from the key file's PEM.
    key_der: Vec<u8>,
    token_uri: String,
    access_token: String,
    expires_at: i64,
}

impl ServiceAccount {
    pub fn load(path: &Path, impersonate: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("đọc key service account {}", path.display()))?;
        let key: KeyFile = serde_json::from_str(&raw).with_context(|| {
            format!("{} không phải JSON key của service account", path.display())
        })?;
        let impersonate = impersonate.trim();
        Ok(Self {
            subject: (!impersonate.is_empty()).then(|| impersonate.to_string()),
            email: key.client_email,
            key_der: pem_to_der(&key.private_key)?,
            token_uri: key.token_uri,
            access_token: String::new(),
            expires_at: 0,
        })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    pub fn access_token(&mut self, http: &reqwest::blocking::Client) -> Result<String> {
        if !self.access_token.is_empty() && now_ts() < self.expires_at {
            return Ok(self.access_token.clone());
        }
        let assertion = self.signed_jwt()?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", GRANT_TYPE)
            .append_pair("assertion", &assertion)
            .finish();
        let resp = http
            .post(&self.token_uri)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .context("gọi token endpoint")?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            bail!(
                "service account bị từ chối ({status}): {}",
                super::oauth::google_error(&text)
            );
        }
        let token: TokenResponse = serde_json::from_str(&text)?;
        self.access_token = token.access_token;
        self.expires_at = now_ts() + token.expires_in - EXPIRY_MARGIN_SECS;
        Ok(self.access_token.clone())
    }

    fn signed_jwt(&self) -> Result<String> {
        let now = now_ts();
        let header = serde_json::json!({ "alg": "RS256", "typ": "JWT" });
        let mut claims = serde_json::json!({
            "iss": self.email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": now,
            "exp": now + TOKEN_LIFETIME,
        });
        if let Some(subject) = &self.subject {
            claims["sub"] = serde_json::Value::String(subject.clone());
        }
        let payload = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature = sign_rs256(&self.key_der, payload.as_bytes())?;
        Ok(format!("{payload}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

fn sign_rs256(key_der: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RsaKeyPair};

    let key =
        RsaKeyPair::from_pkcs8(key_der).map_err(|e| anyhow::anyhow!("key không hợp lệ: {e}"))?;
    let mut signature = vec![0u8; key.public_modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        message,
        &mut signature,
    )
    .map_err(|e| anyhow::anyhow!("ký JWT thất bại: {e}"))?;
    Ok(signature)
}

/// Extracts the DER body of a PEM private key (the key file stores it with
/// literal `\n` escapes already decoded by serde).
fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars().filter(|c| !c.is_whitespace()))
        .collect();
    if body.is_empty() {
        bail!("private_key rỗng");
    }
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("private_key không phải PEM base64 hợp lệ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_to_der_strips_header_and_newlines() {
        let pem = "-----BEGIN PRIVATE KEY-----\nAAEC\nAwQ=\n-----END PRIVATE KEY-----\n";
        assert_eq!(pem_to_der(pem).unwrap(), vec![0x00, 0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn pem_to_der_rejects_empty_key() {
        assert!(pem_to_der("-----BEGIN PRIVATE KEY-----\n-----END PRIVATE KEY-----").is_err());
    }

    #[test]
    fn impersonation_adds_the_sub_claim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.json");
        let key = serde_json::json!({
            "client_email": "bot@proj.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nAAEC\n-----END PRIVATE KEY-----\n",
        });
        std::fs::write(&path, key.to_string()).unwrap();
        let sa = ServiceAccount::load(&path, "  me@company.com ").unwrap();
        assert_eq!(sa.subject(), Some("me@company.com"));
    }

    #[test]
    fn load_rejects_a_non_key_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.json");
        std::fs::write(&path, r#"{"hello":"world"}"#).unwrap();
        assert!(ServiceAccount::load(&path, "").is_err());
    }

    /// Signs with a real key when `QUICK_NOTE_TEST_KEY` points at a service-account
    /// JSON file (no key is committed to the repo).
    #[test]
    fn signs_a_verifiable_jwt_with_a_real_key() {
        let Ok(path) = std::env::var("QUICK_NOTE_TEST_KEY") else {
            return;
        };
        let sa = ServiceAccount::load(Path::new(&path), "").unwrap();
        let jwt = sa.signed_jwt().unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "RS256");
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["iss"], sa.email());
        assert_eq!(claims["scope"], SCOPE);
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(signature.len(), 256, "2048-bit RSA signature");
        std::fs::write(
            "/tmp/qn-jwt-signing-input",
            format!("{}.{}", parts[0], parts[1]),
        )
        .unwrap();
        std::fs::write("/tmp/qn-jwt-signature", &signature).unwrap();
    }

    #[test]
    fn load_reads_email_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.json");
        let key = serde_json::json!({
            "client_email": "bot@proj.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nAAEC\n-----END PRIVATE KEY-----\n",
        });
        std::fs::write(&path, key.to_string()).unwrap();
        let sa = ServiceAccount::load(&path, "").unwrap();
        assert_eq!(sa.email(), "bot@proj.iam.gserviceaccount.com");
        assert_eq!(sa.subject(), None);
        assert_eq!(sa.token_uri, default_token_uri());
        assert_eq!(sa.key_der, vec![0x00, 0x01, 0x02]);
    }
}
