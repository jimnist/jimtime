use super::Command;
use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::cloud::{self, Provider};
use crate::config::Config;
use crate::datarepo::Sync;
use crate::invoice::{self as inv, Status};

/// Copy finalized invoice PDFs to Dropbox and/or Google Drive (optional)
#[derive(Args)]
pub struct Cloud {
    #[command(subcommand)]
    cmd: CloudCmd,
}

#[derive(Subcommand)]
enum CloudCmd {
    /// Connect a provider (one-time browser login; the token goes in the OS keychain)
    Login { provider: Provider },
    /// Forget a provider's token
    Logout { provider: Provider },
    /// Show which providers are configured and logged in
    Status,
    /// Upload an invoice's PDF to every configured provider (retry a failed upload)
    Upload { number: String },
}

#[async_trait::async_trait]
impl Command for Cloud {
    async fn run(&self) -> Result<()> {
        let config = Config::load()?;
        match &self.cmd {
            CloudCmd::Login { provider } => {
                if !provider.is_configured(&config) {
                    bail!(
                        "{} is not configured; add a [{}] section to the config first",
                        provider.name(),
                        provider.config_section()
                    );
                }
                provider.login(&config).await?;
                println!("Logged in to {}. The token is in the OS keychain.", provider.name());
            }
            CloudCmd::Logout { provider } => {
                if provider.logout()? {
                    println!("Removed the {} token from the keychain.", provider.name());
                } else {
                    println!("Not logged in to {}.", provider.name());
                }
            }
            CloudCmd::Status => {
                for p in Provider::ALL {
                    let configured = p.is_configured(&config);
                    let logged_in = p.refresh_token()?.is_some();
                    println!(
                        "{:<13} {}",
                        p.name(),
                        match (configured, logged_in) {
                            (false, _) => "not configured".to_string(),
                            (true, true) => format!("ready ({})", folder(&config, p)),
                            (true, false) =>
                                format!("configured, not logged in (jimtime cloud login {})", p.name()),
                        }
                    );
                }
            }
            CloudCmd::Upload { number } => {
                if !Provider::ALL.iter().any(|p| p.is_configured(&config)) {
                    bail!("no cloud provider is configured");
                }
                let sync = Sync::begin("cloud upload", false)?;
                let mut i = inv::Invoice::load(number)?;
                if i.status == Status::Void {
                    bail!("invoice {number} is void");
                }
                let pdf = i.pdf_path()?;
                let failures = cloud::upload_all(&config, &mut i, &pdf).await;
                i.save()?;
                sync.commit(&format!("invoice: {number} uploaded"))?;
                if let Some((p, e)) = failures.into_iter().next() {
                    return Err(e.context(format!("uploading {number} to {}", p.name())));
                }
            }
        }
        Ok(())
    }
}

fn folder(config: &Config, p: Provider) -> String {
    match p {
        Provider::Dropbox => config.cloud.dropbox.as_ref().map(|d| d.folder.clone()),
        Provider::GoogleDrive => config.cloud.google_drive.as_ref().map(|d| d.folder.clone()),
    }
    .unwrap_or_default()
}
