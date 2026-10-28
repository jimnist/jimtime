//! Dropbox: PKCE "no redirect" login (Dropbox needs exact redirect URIs, so a
//! random loopback port cannot be registered) and file upload.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Pkce, TokenResponse, check};
use crate::config::DropboxSettings;

const AUTHORIZE: &str = "https://www.dropbox.com/oauth2/authorize";
const TOKEN: &str = "https://api.dropboxapi.com/oauth2/token";
const UPLOAD: &str = "https://content.dropboxapi.com/2/files/upload";

pub async fn login(s: &DropboxSettings) -> Result<String> {
    let pkce = Pkce::new()?;
    let mut url = reqwest::Url::parse(AUTHORIZE)?;
    url.query_pairs_mut()
        .append_pair("client_id", &s.app_key)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("token_access_type", "offline");

    println!("Opening Dropbox in your browser. If it does not open, visit:\n\n  {url}\n");
    let _ = crate::invoice::render::open_url(url.as_str());
    let code = prompt("Paste the code Dropbox shows you: ")?;

    let resp = reqwest::Client::new()
        .post(TOKEN)
        .form(&[
            ("code", code.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", &pkce.verifier),
            ("client_id", &s.app_key),
        ])
        .send()
        .await
        .context("exchanging the Dropbox code")?;
    let tok: TokenResponse = check(resp, "Dropbox token exchange").await?.json().await?;
    tok.refresh_token
        .context("Dropbox returned no refresh token (token_access_type=offline was ignored?)")
}

async fn access_token(s: &DropboxSettings, refresh: &str) -> Result<String> {
    let resp = reqwest::Client::new()
        .post(TOKEN)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", &s.app_key),
        ])
        .send()
        .await
        .context("refreshing the Dropbox token")?;
    let tok: TokenResponse = check(
        resp,
        "Dropbox token refresh (log in again with `jimtime cloud login dropbox`)",
    )
    .await?
    .json()
    .await?;
    Ok(tok.access_token)
}

#[derive(Deserialize)]
struct Uploaded {
    path_display: String,
}

pub async fn upload(
    s: &DropboxSettings,
    refresh: &str,
    name: &str,
    bytes: Vec<u8>,
) -> Result<String> {
    let token = access_token(s, refresh).await?;
    let path = format!("{}/{name}", s.folder.trim_end_matches('/'));
    let arg = serde_json::json!({
        "path": path,
        "mode": "overwrite",
        "autorename": false,
        "mute": true,
    });
    let resp = reqwest::Client::new()
        .post(UPLOAD)
        .bearer_auth(token)
        .header("Dropbox-API-Arg", http_header_json(&arg.to_string()))
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .context("uploading to Dropbox")?;
    let done: Uploaded = check(resp, "Dropbox upload").await?.json().await?;
    Ok(done.path_display)
}

/// Dropbox-API-Arg must be ASCII: escape everything else as `\uXXXX`.
fn http_header_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

fn prompt(msg: &str) -> Result<String> {
    use std::io::Write;
    print!("{msg}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim().to_string();
    anyhow::ensure!(!line.is_empty(), "no code entered");
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_json_is_ascii() {
        let s = http_header_json(r#"{"path":"/Rechnungen/Café 😀.pdf"}"#);
        assert!(s.is_ascii());
        assert!(s.contains("Caf\\u00e9"));
        assert!(s.contains("\\ud83d\\ude00"), "surrogate pair");
    }
}
