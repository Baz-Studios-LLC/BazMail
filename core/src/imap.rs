//! IMAP, for the providers that offer nothing better.
//!
//! This is the lossy adapter. JMAP hands us threads, previews, state tokens and
//! a mailbox set per message; IMAP hands us a per-mailbox sequence of UIDs and
//! leaves the rest to us. Everything here exists to project that into the same
//! canonical model the JMAP path produces, so nothing above this file has to
//! know which backend an account came from.
//!
//! Three things bite, and are handled explicitly:
//!
//! - **UIDs are per-mailbox and not stable forever.** They are only meaningful
//!   alongside the mailbox and its `UIDVALIDITY`, so the id we synthesise
//!   carries all three. If the server resets `UIDVALIDITY` the ids change, which
//!   is correct — the old ones genuinely no longer refer to anything.
//! - **There is no preview.** JMAP gives one free; here we fetch a slice of the
//!   body text, or the list looks broken next to a JMAP account.
//! - **There is no threading.** Left for the JWZ pass; every message is
//!   currently its own thread, which is honest rather than wrong.

use crate::model::{EmailAddress, EmailBody, Envelope, Mailbox};
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};

pub const ICLOUD_HOST: &str = "imap.mail.me.com";
pub const ICLOUD_PORT: u16 = 993;

type ImapSession = async_imap::Session<TlsStream<TcpStream>>;

pub struct ImapClient {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl ImapClient {
    pub fn new(host: impl Into<String>, port: u16, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            password: password.into(),
        }
    }

    /// A fresh session per operation.
    ///
    /// Wasteful compared to holding one open, and deliberate for now: a long-
    /// lived IMAP session needs keepalives, reconnection and per-mailbox state,
    /// and that machinery belongs with the IDLE work rather than here.
    async fn session(&self) -> Result<ImapSession> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let server_name = ServerName::try_from(self.host.clone())
            .map_err(|_| anyhow!("invalid IMAP host name: {}", self.host))?;

        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .with_context(|| format!("connecting to {}:{}", self.host, self.port))?;
        let tls = TlsConnector::from(Arc::new(config))
            .connect(server_name, tcp)
            .await
            .context("TLS handshake failed")?;

        let client = async_imap::Client::new(tls);
        client
            .login(&self.username, &self.password)
            .await
            // The tuple carries the client back on failure; only the error matters.
            .map_err(|(e, _)| e)
            .context("IMAP login rejected — for iCloud this must be an app-specific password, not your Apple ID password")
    }

    pub async fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        let mut session = self.session().await?;
        let mut listing = session.list(Some(""), Some("*")).await?;

        // Collected first, because a role can only be resolved once the whole
        // list is known: iCloud exposes both "Sent" and "Sent Messages", and
        // only one of them is the real one.
        let mut listed = Vec::new();
        while let Some(name) = listing.next().await {
            let name = name?;
            if name
                .attributes()
                .iter()
                .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
            {
                continue;
            }
            let path = name.name().to_string();
            listed.push((
                path.clone(),
                display_name(&leaf_name(&path, name.delimiter())),
                special_use_role(name.attributes()),
            ));
        }
        drop(listing);
        let _ = session.logout().await;

        // The server's own SPECIAL-USE declaration wins. A second mailbox whose
        // *name* suggests the same role keeps its place in the list but loses the
        // role, so it sorts as an ordinary folder rather than fighting the real
        // one for the same slot. Nothing is hidden — a folder with mail in it
        // stays reachable, which matters more than a tidy sidebar.
        let mut claimed: Vec<String> = listed.iter().filter_map(|(_, _, r)| r.clone()).collect();

        let mut out = Vec::new();
        for (path, name, special) in listed {
            let role = match special {
                Some(role) => Some(role),
                None => match role_from_name(&path) {
                    Some(guess) if !claimed.contains(&guess) => {
                        claimed.push(guess.clone());
                        Some(guess)
                    }
                    _ => None,
                },
            };
            out.push(Mailbox {
                id: path,
                account_id: account_id.to_string(),
                name,
                role,
                total_emails: 0,
                unread_emails: 0,
                sort_order: 0,
            });
        }
        Ok(out)
    }

    /// The newest `limit` messages in a mailbox.
    ///
    /// Fetched by sequence number counting back from the end rather than by UID
    /// range: UIDs are sparse after deletions, so "the last 200 UIDs" is not
    /// "the last 200 messages".
    pub async fn envelopes(
        &self,
        account_id: &str,
        mailbox: &str,
        limit: usize,
    ) -> Result<Vec<Envelope>> {
        let mut session = self.session().await?;
        let selected = session.select(mailbox).await?;
        let total = selected.exists;
        let uid_validity = selected.uid_validity.unwrap_or(0);

        if total == 0 {
            let _ = session.logout().await;
            return Ok(Vec::new());
        }

        let first = total.saturating_sub(limit as u32 - 1).max(1);
        let range = format!("{first}:{total}");

        // BODY.PEEK leaves \Seen alone — plain BODY[] would mark everything read
        // just by listing the mailbox.
        // Headers come along for the verdict in Authentication-Results. They
        // are a few kilobytes each, which is a real cost, but the same fetch is
        // what References and In-Reply-To will need for threading — so this is
        // paid once rather than twice.
        let query = "(UID ENVELOPE FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.4096>)";
        let mut fetches = session.fetch(&range, query).await?;

        let mut out = Vec::new();
        while let Some(fetch) = fetches.next().await {
            let fetch = fetch?;
            let Some(uid) = fetch.uid else { continue };
            let Some(envelope) = fetch.envelope() else {
                continue;
            };

            let flags: Vec<_> = fetch.flags().collect();
            let is_unread = !flags
                .iter()
                .any(|f| matches!(f, async_imap::types::Flag::Seen));
            let is_flagged = flags
                .iter()
                .any(|f| matches!(f, async_imap::types::Flag::Flagged));

            let received_at = fetch
                .internal_date()
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();

            // The IMAP ENVELOPE hands back RFC 2047 encoded-words verbatim and
            // BODY[TEXT] is still in its transfer encoding, which is why
            // subjects read as "=?utf-8?B?..." and previews as HTML source.
            // mail-parser undoes both, and since the headers are already
            // fetched, re-assembling enough of the message to parse costs
            // nothing more off the wire.
            let raw_prefix: Vec<u8> = fetch
                .header()
                .into_iter()
                .chain(fetch.text())
                .flatten()
                .copied()
                .collect();
            let parsed = mail_parser::MessageParser::default().parse(&raw_prefix);

            let verified_domain = fetch
                .header()
                .and_then(|raw| std::str::from_utf8(raw).ok())
                .and_then(|raw| crate::auth::first_header(raw, "Authentication-Results"))
                .and_then(|header| {
                    crate::auth::parse(&header)
                        .verified_domain()
                        .map(str::to_owned)
                });

            out.push(Envelope {
                id: message_id(mailbox, uid_validity, uid),
                account_id: account_id.to_string(),
                // No server-side threading on IMAP; every message stands alone
                // until the JWZ pass lands.
                thread_id: message_id(mailbox, uid_validity, uid),
                mailbox_ids: vec![mailbox.to_string()],
                verified_domain,
                // Parsed values win; the raw ENVELOPE stays as the fallback so
                // a message mail-parser chokes on still shows something rather
                // than a blank row.
                from: parsed
                    .as_ref()
                    .map(|m| parsed_addresses(m.from()))
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| addresses(envelope.from.as_deref())),
                to: parsed
                    .as_ref()
                    .map(|m| parsed_addresses(m.to()))
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| addresses(envelope.to.as_deref())),
                subject: parsed
                    .as_ref()
                    .and_then(|m| m.subject())
                    .map(str::to_owned)
                    .unwrap_or_else(|| decoded(envelope.subject.as_deref())),
                preview: parsed
                    .as_ref()
                    .map(preview_of)
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| preview_from(fetch.text())),
                received_at,
                is_unread,
                is_flagged,
                message_id: parsed
                    .as_ref()
                    .and_then(|m| m.message_id())
                    .map(str::to_owned),
                references: parsed
                    .as_ref()
                    .and_then(|m| m.references().as_text_list())
                    .map(|list| list.iter().map(|r| r.to_string()).collect())
                    .unwrap_or_default(),
                has_attachment: false,
            });
        }
        drop(fetches);
        let _ = session.logout().await;

        out.sort_by(|a, b| b.received_at.cmp(&a.received_at));
        Ok(out)
    }

    /// Moves a message to another mailbox.
    ///
    /// IMAP has no notion of a message belonging to several mailboxes, so a
    /// "mailbox change" is a move: the JMAP model's add-and-remove collapses to
    /// a single destination. Uses UID MOVE (RFC 6851) where the server has it and
    /// falls back to the copy/flag/expunge dance where it does not — the old way
    /// is not atomic, which is why it is the fallback rather than the default.
    /// Files a copy of a sent message in a mailbox.
    ///
    /// SMTP hands a message to a relay and forgets it — nothing on that path
    /// records that you sent anything. JMAP's submission moves the message into
    /// Sent as part of accepting it; here the copy is ours to append, or mail
    /// leaves the machine with no trace of having done so.
    ///
    /// Flagged `\Seen` because you wrote it: arriving in Sent as unread would
    /// put your own outgoing mail in every unread count you own.
    pub async fn append(&self, mailbox: &str, raw: &[u8]) -> Result<()> {
        let mut session = self.session().await?;
        let result = session
            .append(mailbox, Some(r"(\Seen)"), None, raw)
            .await
            .with_context(|| format!("filing the sent copy in {mailbox}"));
        let _ = session.logout().await;
        result
    }

    pub async fn move_message(&self, id: &str, destination: &str) -> Result<String> {
        let (mailbox, validity, uid) = parse_message_id(id)?;
        let mut session = self.session().await?;
        session.select(&mailbox).await?;

        let uid_set = uid.to_string();
        let moved = session.uid_mv(&uid_set, destination).await;

        if moved.is_err() {
            session
                .uid_copy(&uid_set, destination)
                .await
                .context("copying the message to its destination")?;
            session
                .uid_store(&uid_set, r"+FLAGS (\Deleted)")
                .await
                .context("flagging the original as deleted")?
                .collect::<Vec<_>>()
                .await;
            // Expunge only the message we touched, so a concurrent delete
            // elsewhere in the mailbox is not swept up with it.
            let _ = session.uid_expunge(&uid_set).await;
        }

        let _ = session.logout().await;

        // The UID changes on the way across; the caller needs the new identity
        // because the old one no longer refers to anything.
        Ok(message_id(destination, validity, uid))
    }

    /// Sets or clears flags on a message.
    ///
    /// Takes JMAP keyword names so callers do not have to know which backend
    /// they are talking to; the translation to IMAP's backslash flags happens
    /// here, which is where the protocol difference belongs.
    pub async fn set_flags(&self, id: &str, add: &[String], remove: &[String]) -> Result<()> {
        let (mailbox, _validity, uid) = parse_message_id(id)?;
        let mut session = self.session().await?;
        session.select(&mailbox).await?;

        let uid_set = uid.to_string();
        for (keywords, op) in [(add, "+FLAGS"), (remove, "-FLAGS")] {
            let flags: Vec<&str> = keywords.iter().filter_map(|k| imap_flag(k)).collect();
            if flags.is_empty() {
                continue;
            }
            let query = format!("{op} ({})", flags.join(" "));
            session
                .uid_store(&uid_set, &query)
                .await
                .with_context(|| format!("setting flags on {id}"))?
                .collect::<Vec<_>>()
                .await;
        }

        let _ = session.logout().await;
        Ok(())
    }

    pub async fn body(&self, id: &str) -> Result<EmailBody> {
        let (mailbox, _validity, uid) = parse_message_id(id)?;
        let mut session = self.session().await?;
        session.select(&mailbox).await?;

        let mut fetches = session.uid_fetch(uid.to_string(), "(BODY.PEEK[])").await?;
        let mut raw = Vec::new();
        while let Some(fetch) = fetches.next().await {
            let fetch = fetch?;
            if let Some(body) = fetch.body() {
                raw = body.to_vec();
            }
        }
        drop(fetches);
        let _ = session.logout().await;

        if raw.is_empty() {
            return Err(anyhow!("no message body returned for {id}"));
        }

        // IMAP hands back the raw RFC 5322 message, so unlike JMAP we have to
        // parse MIME ourselves to find the parts worth showing.
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| anyhow!("could not parse the message for {id}"))?;

        Ok(EmailBody {
            id: id.to_string(),
            html: parsed.body_html(0).map(|c| c.into_owned()),
            text: parsed.body_text(0).map(|c| c.into_owned()),
        })
    }
}

/// `mailbox\u{1}uidvalidity\u{1}uid`.
///
/// UIDs mean nothing without the mailbox and its validity, so all three travel
/// together. The separator is a control character precisely because it cannot
/// appear in a mailbox name.
fn message_id(mailbox: &str, uid_validity: u32, uid: u32) -> String {
    format!("{mailbox}\u{1}{uid_validity}\u{1}{uid}")
}

fn imap_flag(keyword: &str) -> Option<&'static str> {
    match keyword {
        "$seen" => Some(r"\Seen"),
        "$flagged" => Some(r"\Flagged"),
        "$answered" => Some(r"\Answered"),
        "$draft" => Some(r"\Draft"),
        _ => None,
    }
}

fn parse_message_id(id: &str) -> Result<(String, u32, u32)> {
    let mut parts = id.split('\u{1}');
    let mailbox = parts.next().unwrap_or_default().to_string();
    let validity = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let uid = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| anyhow!("not an IMAP message id: {id}"))?;
    Ok((mailbox, validity, uid))
}

/// Prefers the server's SPECIAL-USE attributes (RFC 6154) and falls back to
/// names, because plenty of servers advertise neither.
/// The role the server itself declared, via RFC 6154 SPECIAL-USE.
fn special_use_role(attributes: &[async_imap::types::NameAttribute]) -> Option<String> {
    use async_imap::types::NameAttribute;
    for attribute in attributes {
        let role = match attribute {
            NameAttribute::Archive => Some("archive"),
            NameAttribute::Sent => Some("sent"),
            NameAttribute::Drafts => Some("drafts"),
            NameAttribute::Trash => Some("trash"),
            NameAttribute::Junk => Some("junk"),
            NameAttribute::All => Some("all"),
            _ => None,
        };
        if let Some(role) = role {
            return Some(role.to_string());
        }
    }
    None
}

/// A guess from the name, for servers that advertise no SPECIAL-USE at all.
/// Only trusted when the role has not already been claimed by a declaration.
fn role_from_name(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    let leaf = lower.rsplit(['/', '.']).next().unwrap_or(&lower);
    match leaf {
        "inbox" => Some("inbox".into()),
        "archive" => Some("archive".into()),
        "sent" | "sent messages" | "sent items" => Some("sent".into()),
        "drafts" => Some("drafts".into()),
        "trash" | "deleted messages" => Some("trash".into()),
        "junk" | "spam" | "junk email" => Some("junk".into()),
        _ => None,
    }
}

fn leaf_name(path: &str, delimiter: Option<&str>) -> String {
    match delimiter {
        Some(d) if !d.is_empty() => path.rsplit(d).next().unwrap_or(path).to_string(),
        _ => path.to_string(),
    }
}

/// IMAP mandates the inbox be called INBOX and servers shout it; every other
/// name is whatever the user or provider chose, so only this one is rewritten.
/// Title-casing everything would mangle names people picked themselves.
fn display_name(leaf: &str) -> String {
    if leaf.eq_ignore_ascii_case("INBOX") {
        "Inbox".to_string()
    } else {
        leaf.to_string()
    }
}

fn decoded(raw: Option<&[u8]>) -> String {
    raw.map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default()
}

fn addresses(list: Option<&[async_imap::imap_proto::types::Address]>) -> Vec<EmailAddress> {
    list.unwrap_or(&[])
        .iter()
        .map(|a| {
            let mailbox = decoded(a.mailbox.as_deref());
            let host = decoded(a.host.as_deref());
            EmailAddress {
                name: a
                    .name
                    .as_deref()
                    .map(|n| String::from_utf8_lossy(n).to_string())
                    .filter(|n| !n.trim().is_empty()),
                email: if host.is_empty() {
                    mailbox
                } else {
                    format!("{mailbox}@{host}")
                },
            }
        })
        .collect()
}

/// Squashes a raw body slice into one line of preview text.
/// Addresses as mail-parser read them, with RFC 2047 names already decoded.
fn parsed_addresses(address: Option<&mail_parser::Address>) -> Vec<EmailAddress> {
    fn push(out: &mut Vec<EmailAddress>, addr: &mail_parser::Addr) {
        let Some(email) = addr.address.as_deref().filter(|e| !e.trim().is_empty()) else {
            return;
        };
        out.push(EmailAddress {
            name: addr
                .name
                .as_deref()
                .map(str::to_owned)
                .filter(|n| !n.trim().is_empty()),
            email: email.to_string(),
        });
    }

    let mut out = Vec::new();
    match address {
        Some(mail_parser::Address::List(list)) => list.iter().for_each(|a| push(&mut out, a)),
        Some(mail_parser::Address::Group(groups)) => groups
            .iter()
            .flat_map(|g| g.addresses.iter())
            .for_each(|a| push(&mut out, a)),
        None => {}
    }
    out
}

/// A one-line preview from a parsed message.
///
/// Prefers the plain-text alternative and falls back to stripping tags out of
/// the HTML one. Only the body is truncated before parsing, so what arrives
/// here is a real text part rather than the head of a MIME envelope.
fn preview_of(message: &mail_parser::Message) -> String {
    let text = message
        .body_text(0)
        .map(|c| c.into_owned())
        .or_else(|| message.body_html(0).map(|c| strip_tags(&c)))
        .unwrap_or_default();

    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect()
}

/// Removes tags and script/style content, so an HTML-only message previews as
/// its words rather than its markup.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip_to: Option<&str> = None;
    let lower = html.to_ascii_lowercase();

    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if let Some(close) = skip_to {
            // Inside <script> or <style>: nothing here is words.
            if lower[i..].starts_with(close) {
                skip_to = None;
                i += close.len();
            } else {
                i += 1;
            }
            continue;
        }
        match bytes[i] {
            b'<' => {
                if lower[i..].starts_with("<script") {
                    skip_to = Some("</script>");
                } else if lower[i..].starts_with("<style") {
                    skip_to = Some("</style>");
                } else {
                    in_tag = true;
                }
            }
            b'>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(html[i..].chars().next().unwrap_or(' ')),
            _ => {}
        }
        i += html[i..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    out
}

fn preview_from(text: Option<&[u8]>) -> String {
    let Some(bytes) = text else {
        return String::new();
    };
    let raw = String::from_utf8_lossy(bytes);
    let flattened: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    flattened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_subjects_are_decoded() {
        // The exact shape that was reaching the list as literal text.
        let raw = b"Subject: =?utf-8?B?SGVsbG8gdGhlcmU=?=

body
";
        let parsed = mail_parser::MessageParser::default().parse(raw.as_slice()).unwrap();
        assert_eq!(parsed.subject(), Some("Hello there"));
    }

    #[test]
    fn quoted_printable_html_previews_as_words() {
        // Quoted-printable is why previews read as `tml lang=3D"en"`: the =3D
        // is an encoded '=' and nothing was decoding it.
        let raw = b"Content-Type: text/html; charset=utf-8
Content-Transfer-Encoding: quoted-printable

<html lang=3D\"en\"><body><p>Your receipt is ready</p></body></html>
";
        let parsed = mail_parser::MessageParser::default().parse(raw.as_slice()).unwrap();
        let preview = preview_of(&parsed);
        assert!(
            preview.contains("Your receipt is ready"),
            "expected words, got {preview:?}"
        );
        assert!(!preview.contains("3D"), "quoted-printable leaked: {preview:?}");
        assert!(!preview.contains('<'), "markup leaked: {preview:?}");
    }

    #[test]
    fn strip_tags_drops_script_and_style_content() {
        let html = "<style>p{color:red}</style><p>Real text</p><script>alert(1)</script>";
        let text = strip_tags(html);
        assert!(text.contains("Real text"));
        assert!(!text.contains("color"), "style content leaked: {text:?}");
        assert!(!text.contains("alert"), "script content leaked: {text:?}");
    }

    #[test]
    fn message_ids_round_trip() {
        let id = message_id("INBOX", 12345, 678);
        let (mailbox, validity, uid) = parse_message_id(&id).unwrap();
        assert_eq!(mailbox, "INBOX");
        assert_eq!(validity, 12345);
        assert_eq!(uid, 678);
    }

    #[test]
    fn message_ids_survive_mailboxes_with_separators_in_the_name() {
        // Folder names routinely contain the very characters a naive separator
        // would use, which is why the delimiter is a control character.
        let id = message_id("Archive/2026/Q3.notes", 9, 42);
        let (mailbox, _, uid) = parse_message_id(&id).unwrap();
        assert_eq!(mailbox, "Archive/2026/Q3.notes");
        assert_eq!(uid, 42);
    }

    #[test]
    fn roles_fall_back_to_names_when_special_use_is_absent() {
        assert_eq!(role_from_name("INBOX").as_deref(), Some("inbox"));
        assert_eq!(role_from_name("Sent Messages").as_deref(), Some("sent"));
        assert_eq!(role_from_name("Deleted Messages").as_deref(), Some("trash"));
        assert_eq!(role_from_name("Work/Clients"), None);
    }

    #[test]
    fn leaf_name_strips_the_hierarchy() {
        assert_eq!(leaf_name("Archive/2026/Q3", Some("/")), "Q3");
        assert_eq!(leaf_name("INBOX", Some("/")), "INBOX");
    }

    #[test]
    fn only_the_inbox_gets_its_casing_rewritten() {
        assert_eq!(display_name("INBOX"), "Inbox");
        assert_eq!(display_name("inbox"), "Inbox");
        // A folder someone named themselves is left exactly as they named it.
        assert_eq!(display_name("Sent Messages"), "Sent Messages");
        assert_eq!(display_name("URGENT"), "URGENT");
    }

    #[test]
    fn preview_collapses_whitespace() {
        let raw = b"Hello   there\r\n\r\n   from   iCloud\r\n";
        assert_eq!(preview_from(Some(raw)), "Hello there from iCloud");
    }
}
