//! Emailing a finalized invoice over SMTP. The password comes only from
//! `$JIMTIME_SMTP_PASSWORD`. [ADR-0003, ADR-0008]

use anyhow::{Context, Result, bail};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::path::Path;

use super::{Invoice, SendEvent, render};
use crate::config::{Config, EmailSettings, SmtpSecurity};
use crate::timeutil;

pub const PASSWORD_ENV: &str = "JIMTIME_SMTP_PASSWORD";

/// Everything needed to send, checked up front so `finalize` fails before it
/// writes anything rather than after the number is taken.
pub struct Mailer<'a> {
    settings: &'a EmailSettings,
    password: String,
}

impl<'a> Mailer<'a> {
    pub fn from_config(config: &'a Config) -> Result<Mailer<'a>> {
        let settings = config.email.as_ref().context(
            "email is not configured: add an [email] section to the config \
             (or pass --no-send and send later with `jimtime invoice send`)",
        )?;
        let password = match std::env::var(PASSWORD_ENV) {
            Ok(p) if !p.is_empty() => p,
            _ => bail!(
                "{PASSWORD_ENV} is not set.\n\
                 Set your SMTP (app) password in your shell/dotfiles:\n  export {PASSWORD_ENV}=..."
            ),
        };
        settings
            .from
            .parse::<Mailbox>()
            .with_context(|| format!("email.from {:?} is not a valid address", settings.from))?;
        // Never send a password in the clear across a network.
        if settings.security == SmtpSecurity::None && !is_local(&settings.host) {
            bail!(
                "email.security = \"none\" is only allowed for a relay on this machine \
                 (localhost), not {:?}",
                settings.host
            );
        }
        Ok(Mailer { settings, password })
    }

    /// Check an invoice has somewhere to go, with the config key to fix.
    pub fn check_recipients(
        &self,
        config: &Config,
        inv: &Invoice,
        extra_to: &[String],
    ) -> Result<()> {
        let r = inv.recipients(config);
        if r.to.is_empty() && extra_to.is_empty() {
            bail!(
                "client {:?} has no email_to; set it under [clients.{}]",
                inv.client_key,
                inv.client_key
            );
        }
        for a in r.to.iter().chain(&r.cc).chain(&r.bcc).chain(extra_to) {
            a.parse::<Mailbox>()
                .with_context(|| format!("{a:?} is not a valid email address"))?;
        }
        Ok(())
    }

    /// Send the invoice with its PDF attached, returning what to record.
    pub async fn send(
        &self,
        config: &Config,
        inv: &Invoice,
        pdf: &Path,
        extra_to: &[String],
    ) -> Result<SendEvent> {
        let s = self.settings;
        let r = inv.recipients(config);
        let mut to = r.to.clone();
        to.extend(extra_to.iter().cloned());

        let mut b = Message::builder()
            .from(s.from.parse()?)
            .subject(render::text(&s.subject, inv)?.trim().to_string());
        for a in &to {
            b = b.to(a.parse()?);
        }
        for a in &r.cc {
            b = b.cc(a.parse()?);
        }
        for a in &r.bcc {
            b = b.bcc(a.parse()?);
        }
        let bytes = std::fs::read(pdf).with_context(|| format!("reading {}", pdf.display()))?;
        let msg = b.multipart(
            MultiPart::mixed()
                .singlepart(SinglePart::plain(render::text(&s.body, inv)?))
                .singlepart(
                    Attachment::new(inv.pdf_name())
                        .body(bytes, ContentType::parse("application/pdf")?),
                ),
        )?;

        let creds = Credentials::new(s.username.clone(), self.password.clone());
        let transport = match s.security {
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&s.host)?,
            SmtpSecurity::Starttls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&s.host)?
            }
            SmtpSecurity::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&s.host),
        }
        .port(s.port)
        .credentials(creds)
        .build();

        transport
            .send(msg)
            .await
            .with_context(|| format!("sending through {}:{}", s.host, s.port))?;

        Ok(SendEvent {
            at: timeutil::now_rfc3339()?,
            to,
            cc: r.cc,
            bcc: r.bcc,
        })
    }
}

/// Whether an SMTP host is this machine.
fn is_local(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_smtp_is_refused_for_a_remote_host() {
        let cfg = |host: &str| {
            Config::parse(&format!(
                r#"
                [email]
                host = "{host}"
                port = 25
                security = "none"
                username = "u"
                from = "Me <me@example.com>"
                "#
            ))
            .unwrap()
        };
        // SAFETY: this module's tests are the only readers of this variable.
        unsafe { std::env::set_var(PASSWORD_ENV, "x") };
        assert!(Mailer::from_config(&cfg("localhost")).is_ok());
        let err = Mailer::from_config(&cfg("smtp.example.com")).err().unwrap();
        assert!(err.to_string().contains("only allowed"), "{err}");
    }
}
