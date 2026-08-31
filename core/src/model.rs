//! The canonical model.
//!
//! This is JMAP's model, deliberately. Every other backend (IMAP, Gmail, Graph)
//! projects *into* these types via its own adapter rather than the other way
//! round — JMAP is the only mail protocol designed as a superset of the others,
//! so adopting it here keeps the Fastmail path lossless and confines the pain to
//! the adapters that genuinely need it.

use serde::{Deserialize, Serialize};

/// A configured account. `id` is ours, not the backend's.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// Shown in the sidebar, e.g. "example.com".
    pub label: String,
    /// The provenance colour carried on avatar rings in the UI.
    pub color: String,
    /// Address mail is sent from by default.
    pub identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mailbox {
    pub id: String,
    pub account_id: String,
    pub name: String,
    /// JMAP role: "inbox", "archive", "drafts", "sent", "trash", "junk".
    pub role: Option<String>,
    pub total_emails: u32,
    pub unread_emails: u32,
    pub sort_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    pub name: Option<String>,
    pub email: String,
}

impl EmailAddress {
    /// What the list row shows: a display name when we have one, otherwise the
    /// local part, which reads better than a full address in a narrow column.
    pub fn display(&self) -> String {
        if let Some(name) = self.name.as_ref().filter(|n| !n.trim().is_empty()) {
            return name.clone();
        }
        self.email
            .split('@')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.email)
            .to_string()
    }
}

/// Everything a list row needs, and nothing more. Bodies are fetched separately
/// so that scrolling a mailbox never pulls megabytes of HTML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub mailbox_ids: Vec<String>,
    pub from: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub subject: String,
    pub preview: String,
    /// RFC 3339, as JMAP gives it. Formatting is the UI's problem.
    pub received_at: String,
    pub is_unread: bool,
    pub is_flagged: bool,
    pub has_attachment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailBody {
    pub id: String,
    pub html: Option<String>,
    pub text: Option<String>,
}

/// Where a thread sits in the triage model — computed, never stored on the
/// server. See `triage.rs` for how these are decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Lane {
    NeedsYou,
    WaitingOn,
    Fyi,
}

/// A change made locally that still has to reach the server.
///
/// Mutations are applied to the mirror first and sent afterwards, so the UI
/// never waits on the network. The queue is what makes that safe: if the send
/// fails, the change is still recorded and gets retried rather than silently
/// diverging from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mutation {
    pub id: i64,
    pub account_id: String,
    pub email_id: String,
    /// What the user did, for display and for undo. "archive" today.
    pub kind: String,
    pub add_mailbox_ids: Vec<String>,
    pub remove_mailbox_ids: Vec<String>,
    /// Keyword changes, named the JMAP way (`$seen`, `$flagged`). The IMAP
    /// adapter translates them to their `\Seen`-style equivalents.
    pub add_flags: Vec<String>,
    pub remove_flags: Vec<String>,
    pub attempts: u32,
}
