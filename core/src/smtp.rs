//! Submission over SMTP, for accounts that read over IMAP.
//!
//! IMAP has no send verb. An account reached that way needs a second protocol
//! and a second host entirely, which is why sending was missing here long after
//! it worked over JMAP.
//!
//! Two things differ from the JMAP path and both matter:
//!
//! - **Nothing files the sent copy.** JMAP's `EmailSubmission` moves the message
//!   into Sent as part of accepting it. SMTP hands the message to a relay and
//!   forgets it, so the copy in Sent is ours to append over IMAP afterwards —
//!   otherwise mail leaves the machine with no record that it ever did.
//! - **We build the message ourselves.** JMAP takes structured fields and
//!   assembles RFC 5322 server-side. Here the bytes on the wire are exactly the
//!   bytes we produce, which is also what makes the Sent copy honest: the same
//!   buffer is submitted and appended, so the two cannot drift.

use crate::model::{EmailAddress, Outgoing};
use anyhow::{Context, Result};
use mail_send::mail_builder::MessageBuilder;
use mail_send::smtp::message::{Address, Message};
use mail_send::SmtpClientBuilder;
use std::borrow::Cow;

pub struct SmtpClient {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl SmtpClient {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            password: password.into(),
        }
    }

    /// Builds the RFC 5322 message this account would send.
    ///
    /// Separate from sending so the identical bytes can be appended to Sent.
    /// Building it twice would risk a Sent copy that differs from what the
    /// recipient got — a different Date or Message-ID is enough to break
    /// threading against your own replies.
    pub fn compose(&self, from: &EmailAddress, message: &Outgoing) -> Result<Vec<u8>> {
        let pairs = |list: &[EmailAddress]| -> Vec<(String, String)> {
            list.iter()
                .map(|a| (a.name.clone().unwrap_or_default(), a.email.clone()))
                .collect()
        };

        let mut builder = MessageBuilder::new()
            .from((
                from.name.clone().unwrap_or_default(),
                from.email.clone(),
            ))
            .to(pairs(&message.to))
            .subject(message.subject.clone())
            .text_body(message.text.clone());

        if !message.cc.is_empty() {
            builder = builder.cc(pairs(&message.cc));
        }
        // Bcc is deliberately not written into the message. It goes to the
        // relay as an envelope recipient and nowhere else — a Bcc header on the
        // wire is how a blind copy stops being blind.
        if let Some(parent) = &message.in_reply_to {
            builder = builder.in_reply_to(parent.clone());
        }
        if !message.references.is_empty() {
            builder = builder.references(message.references.clone());
        }

        builder
            .write_to_vec()
            .context("building the message to send")
    }

    /// Submits a prepared message.
    ///
    /// Every recipient — To, Cc and Bcc alike — is an envelope recipient, which
    /// is what actually decides delivery. The headers only decide what the
    /// reader sees.
    pub async fn send(
        &self,
        from: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> Result<()> {
        if recipients.is_empty() {
            anyhow::bail!("a message needs at least one recipient");
        }

        let mut client = SmtpClientBuilder::new(self.host.clone(), self.port)
            .map_err(|e| anyhow::anyhow!("{} is not a usable mail host: {e}", self.host))?
            // 465 is implicit TLS; 587 upgrades with STARTTLS. Certificate
            // validation stays on either way — allow_invalid_certs is never
            // called, so a failed upgrade is an error rather than a quiet
            // plaintext send of someone's mail.
            .implicit_tls(self.port == 465)
            .credentials((self.username.clone(), self.password.clone()))
            .connect()
            .await
            .with_context(|| format!("connecting to {}:{} to send", self.host, self.port))?;

        // Envelope recipients, which is what actually decides delivery. The
        // headers only decide what the reader sees — which is how Bcc reaches
        // this list without ever reaching the message.
        let message = Message {
            mail_from: Address {
                email: Cow::Borrowed(from),
                parameters: Default::default(),
            },
            rcpt_to: recipients
                .iter()
                .map(|to| Address {
                    email: Cow::Borrowed(to.as_str()),
                    parameters: Default::default(),
                })
                .collect(),
            body: Cow::Borrowed(raw),
        };

        client
            .send(message)
            .await
            .context("the mail server refused the message")?;
        Ok(())
    }
}
