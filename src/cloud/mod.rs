//! Uploading finalized invoice PDFs to Dropbox and Google Drive. [ADR-0010]
//!
//! Each provider is optional and on only when its config section exists. Auth
//! is a one-time OAuth login with PKCE; the long-lived refresh token lives in
//! the OS keychain, never on disk.

mod dropbox;
mod gdrive;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::ValueEnum;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::config::Config;
use crate::invoice::{Invoice, Upload};
use crate::timeutil;

const KEYCHAIN_SERVICE: &str = "jimtime";

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum Provider {
    Dropbox,
    GoogleDrive,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Dropbox, Provider::GoogleDrive];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Dropbox => "dropbox",
            Provider::GoogleDrive => "google-drive",
        }
    }

    /// The config table that turns this provider on.
    pub fn config_section(self) -> &'static str {
        match self {
            Provider::Dropbox => "cloud.dropbox",
            Provider::GoogleDrive => "cloud.google_drive",
        }
    }

    pub fn is_configured(self, config: &Config) -> bool {
        match self {
            Provider::Dropbox => config.cloud.dropbox.is_some(),
            Provider::GoogleDrive => config.cloud.google_drive.is_some(),
        }
    }

    fn keychain(self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, self.name()).context("opening the OS keychain")
    }

    /// The stored refresh token, if logged in.
    pub fn refresh_token(self) -> Result<Option<String>> {
        match self.keychain()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e).context("reading the refresh token from the OS keychain"),
        }
    }

    fn store_refresh_token(self, token: &str) -> Result<()> {
        self.keychain()?
            .set_password(token)
            .context("saving the refresh token to the OS keychain")
    }

    pub fn logout(self) -> Result<bool> {
        match self.keychain()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e).context("removing the refresh token from the OS keychain"),
        }
    }

    /// Run the interactive OAuth login and store the refresh token.
    pub async fn login(self, config: &Config) -> Result<()> {
        let token = match self {
            Provider::Dropbox => dropbox::login(settings_dropbox(config)?).await?,
            Provider::GoogleDrive => gdrive::login(settings_gdrive(config)?).await?,
        };
        self.store_refresh_token(&token)
    }

    /// Upload a PDF under `name` into the configured folder.
    pub async fn upload(self, config: &Config, pdf: &Path, name: &str) -> Result<Upload> {
        let refresh = self.refresh_token()?.with_context(|| {
            format!(
                "not logged in to {0}; run `jimtime cloud login {0}`",
                self.name()
            )
        })?;
        let bytes = std::fs::read(pdf).with_context(|| format!("reading {}", pdf.display()))?;
        let location = match self {
            Provider::Dropbox => {
                dropbox::upload(settings_dropbox(config)?, &refresh, name, bytes).await?
            }
            Provider::GoogleDrive => {
                gdrive::upload(settings_gdrive(config)?, &refresh, name, bytes).await?
            }
        };
        Ok(Upload {
            provider: self.name().to_string(),
            at: timeutil::now_rfc3339()?,
            location,
        })
    }
}

fn settings_dropbox(config: &Config) -> Result<&crate::config::DropboxSettings> {
    config
        .cloud
        .dropbox
        .as_ref()
        .context("Dropbox is not configured: add a [cloud.dropbox] section")
}

fn settings_gdrive(config: &Config) -> Result<&crate::config::GoogleDriveSettings> {
    config
        .cloud
        .google_drive
        .as_ref()
        .context("Google Drive is not configured: add a [cloud.google_drive] section")
}

/// Upload an invoice's PDF to every configured provider, recording each
/// success on the invoice. Failures are returned, not raised: an upload never
/// fails an invoice that has already been issued. [ADR-0010]
pub async fn upload_all(
    config: &Config,
    inv: &mut Invoice,
    pdf: &Path,
) -> Vec<(Provider, anyhow::Error)> {
    let mut failures = Vec::new();
    for p in Provider::ALL {
        if !p.is_configured(config) {
            continue;
        }
        match p.upload(config, pdf, &inv.pdf_name()).await {
            Ok(u) => {
                println!("Uploaded to {}: {}", p.name(), u.location);
                inv.uploads.push(u);
            }
            Err(e) => failures.push((p, e)),
        }
    }
    failures
}

/// A PKCE verifier and its S256 challenge.
pub(crate) struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Result<Pkce> {
        let mut buf = [0u8; 48];
        getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
        let verifier = URL_SAFE_NO_PAD.encode(buf);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(Pkce {
            verifier,
            challenge,
        })
    }
}

/// A random URL-safe token (OAuth `state`).
pub(crate) fn random_token() -> Result<String> {
    let mut buf = [0u8; 18];
    getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Fail with the provider's error body on a non-2xx response.
pub(crate) async fn check(resp: reqwest::Response, what: &str) -> Result<reqwest::Response> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("{what} failed with {status}\n{body}");
    }
    Ok(resp)
}

#[derive(serde::Deserialize)]
pub(crate) struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_the_verifier() {
        let p = Pkce::new().unwrap();
        assert!((43..=128).contains(&p.verifier.len()), "RFC 7636 length");
        assert_eq!(
            p.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes()))
        );
        assert_ne!(Pkce::new().unwrap().verifier, p.verifier);
    }

    /// keyring falls back to an in-memory mock store when no platform store
    /// is compiled in, which would make every login silently forgotten. A
    /// value written through one handle must be readable through a fresh one,
    /// and on macOS must be visible to `security`. Touches the real keychain
    /// (under a throwaway service name), so it only runs on request:
    /// `cargo test keychain -- --ignored`.
    #[test]
    #[ignore]
    fn keychain_persists_beyond_the_handle() {
        let service = "jimtime-selftest";
        let write = keyring::Entry::new(service, "probe").unwrap();
        write.set_password("s3cret").unwrap();

        let read = keyring::Entry::new(service, "probe").unwrap();
        let got = read.get_password();
        #[cfg(target_os = "macos")]
        let seen_by_os = std::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-a", "probe"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        read.delete_credential().unwrap();

        assert_eq!(
            got.unwrap(),
            "s3cret",
            "a fresh handle sees it: not the mock store"
        );
        #[cfg(target_os = "macos")]
        assert!(seen_by_os, "the item is in the macOS keychain");
        assert!(matches!(
            keyring::Entry::new(service, "probe")
                .unwrap()
                .get_password(),
            Err(keyring::Error::NoEntry)
        ));
    }
}
