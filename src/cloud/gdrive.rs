//! Google Drive: PKCE login through a loopback redirect (allowed for desktop
//! clients on any port), and upload into a folder path under My Drive. Access
//! is `drive.file`: jimtime sees only what it created.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Pkce, TokenResponse, check, random_token};
use crate::config::GoogleDriveSettings;

const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
pub const SECRET_ENV: &str = "JIMTIME_GDRIVE_CLIENT_SECRET";

fn client_secret() -> Result<String> {
    match std::env::var(SECRET_ENV) {
        Ok(s) if !s.is_empty() => Ok(s),
        _ => bail!(
            "{SECRET_ENV} is not set.\n\
             Set the client secret of your Google \"Desktop app\" OAuth client:\n  export {SECRET_ENV}=..."
        ),
    }
}

pub async fn login(s: &GoogleDriveSettings) -> Result<String> {
    let secret = client_secret()?;
    let pkce = Pkce::new()?;
    let state = random_token()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("opening a local port for the Google redirect")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let mut url = reqwest::Url::parse(AUTHORIZE)?;
    url.query_pairs_mut()
        .append_pair("client_id", &s.client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);
    println!("Opening Google in your browser. If it does not open, visit:\n\n  {url}\n");
    let _ = crate::invoice::render::open_url(url.as_str());
    println!("Waiting for you to approve access...");

    let code = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        receive_code(&listener, &state),
    )
    .await
    .context("timed out waiting for the Google login")??;

    let resp = reqwest::Client::new()
        .post(TOKEN)
        .form(&[
            ("code", code.as_str()),
            ("client_id", &s.client_id),
            ("client_secret", &secret),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
            ("code_verifier", &pkce.verifier),
        ])
        .send()
        .await
        .context("exchanging the Google code")?;
    let tok: TokenResponse = check(resp, "Google token exchange").await?.json().await?;
    tok.refresh_token
        .context("Google returned no refresh token")
}

/// Accept the browser's redirect, answer it, and return the code.
async fn receive_code(listener: &tokio::net::TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let Some(target) = req.lines().next().and_then(|l| l.split_whitespace().nth(1)) else {
            continue;
        };
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
        let q = |k: &str| {
            url.query_pairs()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.into_owned())
        };
        // Browsers also ask for /favicon.ico; ignore anything without our params.
        if q("state").is_none() && q("error").is_none() {
            let _ = sock
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let (ok, msg) = match (q("state"), q("code"), q("error")) {
            (_, _, Some(err)) => (
                Err(anyhow::anyhow!("Google login failed: {err}")),
                "Login failed. You can close this tab.",
            ),
            (Some(s), Some(code), None) if s == state => (
                Ok(code),
                "jimtime is connected to Google Drive. You can close this tab.",
            ),
            _ => (
                Err(anyhow::anyhow!("Google login returned an unexpected state")),
                "Login failed. You can close this tab.",
            ),
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>jimtime</title>\
             <body style=\"font:16px -apple-system,sans-serif;margin:4em\">{msg}</body>"
        );
        let _ = sock
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        return ok;
    }
}

async fn access_token(s: &GoogleDriveSettings, refresh: &str) -> Result<String> {
    let secret = client_secret()?;
    let resp = reqwest::Client::new()
        .post(TOKEN)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", s.client_id.as_str()),
            ("client_secret", secret.as_str()),
        ])
        .send()
        .await
        .context("refreshing the Google token")?;
    let tok: TokenResponse = check(
        resp,
        "Google token refresh (log in again with `jimtime cloud login google-drive`)",
    )
    .await?
    .json()
    .await?;
    Ok(tok.access_token)
}

#[derive(Deserialize)]
struct FileList {
    files: Vec<FileRef>,
}

#[derive(Deserialize)]
struct FileRef {
    id: String,
}

/// Escape a value for a Drive `q` string literal.
fn q_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

async fn find(
    http: &reqwest::Client,
    token: &str,
    name: &str,
    parent: &str,
    folder: bool,
) -> Result<Option<String>> {
    let mut q = format!(
        "name = '{}' and '{}' in parents and trashed = false",
        q_escape(name),
        q_escape(parent)
    );
    if folder {
        q.push_str(&format!(" and mimeType = '{FOLDER_MIME}'"));
    }
    let resp = http
        .get(FILES)
        .bearer_auth(token)
        .query(&[
            ("q", q.as_str()),
            ("fields", "files(id)"),
            ("spaces", "drive"),
        ])
        .send()
        .await
        .context("searching Google Drive")?;
    let list: FileList = check(resp, "Google Drive search").await?.json().await?;
    Ok(list.files.into_iter().next().map(|f| f.id))
}

/// Walk (creating as needed) a `/`-separated folder path under My Drive.
async fn ensure_folder(http: &reqwest::Client, token: &str, path: &str) -> Result<String> {
    let mut parent = "root".to_string();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        parent = match find(http, token, part, &parent, true).await? {
            Some(id) => id,
            None => {
                let resp = http
                    .post(FILES)
                    .bearer_auth(token)
                    .query(&[("fields", "id")])
                    .json(&serde_json::json!({
                        "name": part, "mimeType": FOLDER_MIME, "parents": [parent]
                    }))
                    .send()
                    .await
                    .context("creating a Google Drive folder")?;
                let f: FileRef = check(resp, "Google Drive folder create")
                    .await?
                    .json()
                    .await?;
                f.id
            }
        };
    }
    Ok(parent)
}

pub async fn upload(
    s: &GoogleDriveSettings,
    refresh: &str,
    name: &str,
    bytes: Vec<u8>,
) -> Result<String> {
    let token = access_token(s, refresh).await?;
    let http = reqwest::Client::new();
    let folder = ensure_folder(&http, &token, &s.folder).await?;

    // Re-uploading replaces the file rather than making a second copy.
    if let Some(id) = find(&http, &token, name, &folder, false).await? {
        let resp = http
            .patch(format!("{UPLOAD}/{id}"))
            .bearer_auth(&token)
            .query(&[("uploadType", "media")])
            .header("Content-Type", "application/pdf")
            .body(bytes)
            .send()
            .await
            .context("updating the file on Google Drive")?;
        check(resp, "Google Drive update").await?;
        return Ok(format!(
            "{}/{name} (id {id})",
            s.folder.trim_end_matches('/')
        ));
    }

    let boundary = format!("jimtime-{}", random_token()?);
    let meta = serde_json::json!({ "name": name, "parents": [folder] }).to_string();
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n\
             --{boundary}\r\nContent-Type: application/pdf\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = http
        .post(UPLOAD)
        .bearer_auth(&token)
        .query(&[("uploadType", "multipart"), ("fields", "id")])
        .header(
            "Content-Type",
            format!("multipart/related; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .context("uploading to Google Drive")?;
    let f: FileRef = check(resp, "Google Drive upload").await?.json().await?;
    Ok(format!(
        "{}/{name} (id {})",
        s.folder.trim_end_matches('/'),
        f.id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_query_literals_are_escaped() {
        assert_eq!(q_escape(r"Jim's \ Invoices"), r"Jim\'s \\ Invoices");
    }

    /// Play the browser: request a path on the loopback server, return the
    /// HTTP response text.
    async fn browser_get(port: u16, path: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn loopback_ignores_favicon_then_returns_the_code() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { receive_code(&listener, "st8").await });

        let favicon = browser_get(port, "/favicon.ico").await;
        assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
        let page = browser_get(port, "/?state=st8&code=abc%2F123&scope=x").await;
        assert!(page.contains("connected to Google Drive"), "{page}");

        assert_eq!(server.await.unwrap().unwrap(), "abc/123", "decoded");
    }

    #[tokio::test]
    async fn loopback_rejects_a_forged_state_and_reports_denial() {
        for (path, want) in [
            ("/?state=WRONG&code=abc", "unexpected state"),
            ("/?error=access_denied&state=st8", "access_denied"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move { receive_code(&listener, "st8").await });
            let page = browser_get(port, path).await;
            assert!(page.contains("Login failed"), "{page}");
            let err = server.await.unwrap().unwrap_err();
            assert!(err.to_string().contains(want), "{err}");
        }
    }
}
