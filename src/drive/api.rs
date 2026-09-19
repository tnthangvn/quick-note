//! Minimal Google Drive v3 REST client (folder lookup, upload, download).

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::Deserialize;
use url::Url;

use super::oauth::Session;

const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

#[derive(Debug, Clone, Deserialize)]
pub struct DriveFile {
    pub id: String,
}

#[derive(Deserialize)]
struct FileList {
    files: Vec<DriveFile>,
}

pub struct Drive<'a> {
    http: &'a Client,
    session: &'a mut Session,
}

impl<'a> Drive<'a> {
    pub fn new(http: &'a Client, session: &'a mut Session) -> Self {
        Self { http, session }
    }

    fn send(&mut self, req: RequestBuilder) -> Result<Response> {
        let token = self.session.access_token(self.http)?;
        let resp = req.bearer_auth(token).send().context("gọi Drive API")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("Drive API lỗi {status}: {}", api_error(&body));
        }
        Ok(resp)
    }

    fn list(&mut self, query: &str) -> Result<Vec<DriveFile>> {
        let mut url = Url::parse(FILES_URL)?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("fields", "files(id)")
            .append_pair("orderBy", "modifiedTime desc")
            .append_pair("pageSize", "100")
            .append_pair("spaces", "drive");
        let req = self.http.get(url);
        Ok(self.send(req)?.json::<FileList>()?.files)
    }

    /// Returns the folder id: the configured one, or find-or-create by name in My Drive root.
    pub fn ensure_folder(&mut self, folder_id: &str, folder_name: &str) -> Result<String> {
        if !folder_id.trim().is_empty() {
            let mut url = Url::parse(&format!("{FILES_URL}/{}", folder_id.trim()))?;
            url.query_pairs_mut()
                .append_pair("fields", "id,name,mimeType");
            let req = self.http.get(url);
            let meta: serde_json::Value = self.send(req)?.json()?;
            if meta.get("mimeType").and_then(|m| m.as_str()) != Some(FOLDER_MIME) {
                bail!("folder_id không phải là thư mục Drive");
            }
            return Ok(folder_id.trim().to_string());
        }
        let q = format!(
            "name = '{}' and mimeType = '{FOLDER_MIME}' and 'root' in parents and trashed = false",
            escape_query(folder_name)
        );
        if let Some(found) = self.list(&q)?.into_iter().next() {
            return Ok(found.id);
        }
        let body = serde_json::json!({ "name": folder_name, "mimeType": FOLDER_MIME, "parents": ["root"] });
        let req = self.http.post(format!("{FILES_URL}?fields=id")).json(&body);
        let created: serde_json::Value = self.send(req)?.json()?;
        created["id"]
            .as_str()
            .map(str::to_owned)
            .context("Drive không trả id thư mục")
    }

    pub fn find_in_folder(&mut self, folder_id: &str, name: &str) -> Result<Option<DriveFile>> {
        let q = format!(
            "name = '{}' and '{}' in parents and trashed = false",
            escape_query(name),
            escape_query(folder_id)
        );
        Ok(self.list(&q)?.into_iter().next())
    }

    /// Creates or overwrites `name` inside the folder.
    pub fn upsert(
        &mut self,
        folder_id: &str,
        name: &str,
        mime: &str,
        content: Vec<u8>,
    ) -> Result<String> {
        if let Some(existing) = self.find_in_folder(folder_id, name)? {
            let url = format!("{UPLOAD_URL}/{}?uploadType=media&fields=id", existing.id);
            let req = self
                .http
                .patch(url)
                .header("Content-Type", mime)
                .body(content);
            self.send(req)?;
            return Ok(existing.id);
        }
        let metadata =
            serde_json::json!({ "name": name, "parents": [folder_id], "mimeType": mime });
        let (boundary, body) = multipart_related(&metadata, mime, &content);
        let req = self
            .http
            .post(format!("{UPLOAD_URL}?uploadType=multipart&fields=id"))
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body);
        let created: serde_json::Value = self.send(req)?.json()?;
        created["id"]
            .as_str()
            .map(str::to_owned)
            .context("Drive không trả id file")
    }

    pub fn download(&mut self, file_id: &str) -> Result<Vec<u8>> {
        let req = self.http.get(format!("{FILES_URL}/{file_id}?alt=media"));
        Ok(self.send(req)?.bytes()?.to_vec())
    }
}

/// Escapes a value for use inside a single-quoted Drive query string.
pub fn escape_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn multipart_related(
    metadata: &serde_json::Value,
    mime: &str,
    content: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!("qn-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(content.len() + 512);
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

fn api_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_query_handles_quotes_and_backslashes() {
        assert_eq!(escape_query(r"O'Neil\notes"), r"O\'Neil\\notes");
    }

    #[test]
    fn multipart_body_has_both_parts() {
        let meta = serde_json::json!({"name": "a.md"});
        let (boundary, body) = multipart_related(&meta, "text/markdown", b"hello");
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with(&format!("--{boundary}\r\n")));
        assert!(text.contains(r#"{"name":"a.md"}"#));
        assert!(text.contains("Content-Type: text/markdown\r\n\r\nhello\r\n"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn api_error_prefers_message() {
        assert_eq!(
            api_error(r#"{"error":{"message":"File not found"}}"#),
            "File not found"
        );
    }
}
