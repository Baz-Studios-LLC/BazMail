//! A small JMAP client — only the subset BazMail needs.
//!
//! Hand-rolled rather than pulled from a crate on purpose: the surface we use is
//! four method calls, JMAP is plain JSON over HTTP, and owning it means the
//! canonical model in `model.rs` never has to bend around somebody else's types.

use crate::model::{EmailAddress, EmailBody, Envelope, Mailbox};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const CAP_CORE: &str = "urn:ietf:params:jmap:core";
const CAP_MAIL: &str = "urn:ietf:params:jmap:mail";
/// Sending is a separate capability from reading, and asking for it on every
/// request would make a server that cannot send refuse the ones that read.
const CAP_SUBMISSION: &str = "urn:ietf:params:jmap:submission";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub api_url: String,
    /// The address the token belongs to. Used to prefill the account's identity
    /// so nobody has to retype what the server already knows.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub primary_accounts: HashMap<String, String>,
    #[serde(default)]
    pub accounts: HashMap<String, AccountInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(default)]
    pub name: String,
}

impl Session {
    /// The backend's own account id — distinct from our `Account::id`.
    pub fn mail_account_id(&self) -> Result<&str> {
        self.primary_accounts
            .get(CAP_MAIL)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("JMAP session exposes no primary mail account"))
    }

    /// The account that sends.
    ///
    /// Usually the same as the mail account, but the spec keeps them separate
    /// and a server is entitled to differ. Falling back to the mail account is
    /// a guess, which is why it is only a fallback — and an account with
    /// neither genuinely cannot send, so that is an error rather than a
    /// silently wrong id.
    pub fn submission_account_id(&self) -> Result<&str> {
        self.primary_accounts
            .get(CAP_SUBMISSION)
            .or_else(|| self.primary_accounts.get(CAP_MAIL))
            .map(String::as_str)
            .ok_or_else(|| {
                anyhow!("this account cannot send: the session offers no submission account")
            })
    }
}

pub struct JmapClient {
    http: reqwest::Client,
    token: String,
    session: Session,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JmapRequest<'a> {
    using: &'a [&'a str],
    method_calls: Vec<Value>,
}

/// One of an account's sending identities, as `Identity/get` returns it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapIdentity {
    id: String,
    #[serde(default)]
    email: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapResponse {
    method_responses: Vec<Value>,
}

impl JmapResponse {
    /// Pulls the arguments of the response tagged with `call_id`, turning a JMAP
    /// `error` response into a real error rather than a confusing parse failure.
    fn args(&self, call_id: &str) -> Result<&Value> {
        for entry in &self.method_responses {
            let name = entry.get(0).and_then(Value::as_str).unwrap_or_default();
            let id = entry.get(2).and_then(Value::as_str).unwrap_or_default();
            if id != call_id {
                continue;
            }
            let args = entry
                .get(1)
                .ok_or_else(|| anyhow!("malformed JMAP response for '{call_id}'"))?;
            if name == "error" {
                let kind = args
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                let detail = args.get("description").and_then(Value::as_str).unwrap_or("");
                anyhow::bail!("JMAP error on '{call_id}': {kind} {detail}");
            }
            return Ok(args);
        }
        Err(anyhow!("no JMAP response tagged '{call_id}'"))
    }
}

impl JmapClient {
    pub async fn connect(http: reqwest::Client, session_url: &str, token: String) -> Result<Self> {
        let session: Session = http
            .get(session_url)
            .bearer_auth(&token)
            .send()
            .await
            .context("requesting JMAP session")?
            .error_for_status()
            .context("JMAP session rejected — check the API token and its scopes")?
            .json()
            .await
            .context("parsing JMAP session")?;
        Ok(Self {
            http,
            token,
            session,
        })
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    async fn call(&self, method_calls: Vec<Value>) -> Result<JmapResponse> {
        self.call_using(&[CAP_CORE, CAP_MAIL], method_calls).await
    }

    /// Reading and sending are separate JMAP capabilities, and a server only
    /// accepts a request whose `using` it can satisfy — so asking for
    /// submission on every call would make a read-only account fail to read.
    async fn call_using(&self, using: &[&str], method_calls: Vec<Value>) -> Result<JmapResponse> {
        let body = JmapRequest {
            using,
            method_calls,
        };
        let response = self
            .http
            .post(&self.session.api_url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("sending JMAP request")?;

        // The body is where the server says *what* it objected to — an unknown
        // property, an unsupported capability, a malformed argument. Dropping
        // it and reporting only "rejected" turns a one-line fix into a guessing
        // game, which is exactly what happened here.
        let status = response.status();
        if !status.is_success() {
            let detail = response
                .text()
                .await
                .unwrap_or_else(|_| "<no response body>".into());
            anyhow::bail!("JMAP request rejected ({status}): {detail}");
        }

        Ok(response.json().await.context("parsing JMAP response")?)
    }

    pub async fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        let backend_account = self.session.mail_account_id()?;
        let response = self
            .call(vec![json!([
                "Mailbox/get",
                { "accountId": backend_account, "ids": Value::Null },
                "m0"
            ])])
            .await?;

        let list = response.args("m0")?.get("list").cloned().unwrap_or(json!([]));
        let raw: Vec<JmapMailbox> = serde_json::from_value(list).context("parsing mailboxes")?;

        Ok(raw
            .into_iter()
            .map(|m| Mailbox {
                id: m.id,
                account_id: account_id.to_string(),
                name: m.name,
                role: m.role,
                total_emails: m.total_emails,
                unread_emails: m.unread_emails,
                sort_order: m.sort_order,
            })
            .collect())
    }

    /// One round trip: query the mailbox, then fetch the envelopes of whatever
    /// came back, using a JMAP back-reference so the ids never touch our process.
    pub async fn envelopes(
        &self,
        account_id: &str,
        mailbox_id: &str,
        limit: usize,
    ) -> Result<Vec<Envelope>> {
        let backend_account = self.session.mail_account_id()?;
        let response = self
            .call(vec![
                json!([
                    "Email/query",
                    {
                        "accountId": backend_account,
                        "filter": { "inMailbox": mailbox_id },
                        "sort": [{ "property": "receivedAt", "isAscending": false }],
                        "collapseThreads": true,
                        "limit": limit
                    },
                    "q0"
                ]),
                json!([
                    "Email/get",
                    {
                        "accountId": backend_account,
                        "#ids": { "resultOf": "q0", "name": "Email/query", "path": "/ids" },
                        "properties": [
                            "id", "threadId", "mailboxIds", "keywords", "from", "to",
                            "subject", "receivedAt", "preview", "hasAttachment",
                            "messageId", "references",
                            // :all, not the default. JMAP returns the *last*
                            // header of a given name, and a message can carry
                            // several Authentication-Results — including ones
                            // the sender wrote. The provider's is the topmost,
                            // so we need the whole list to take the first.
                            "header:Authentication-Results:asText:all"
                        ]
                    },
                    "g0"
                ]),
            ])
            .await?;

        let list = response.args("g0")?.get("list").cloned().unwrap_or(json!([]));
        let raw: Vec<JmapEmail> = serde_json::from_value(list).context("parsing envelopes")?;

        let mut envelopes: Vec<Envelope> = raw
            .into_iter()
            .map(|e| e.into_envelope(account_id))
            .collect();
        // Email/get does not promise query order back.
        envelopes.sort_by(|a, b| b.received_at.cmp(&a.received_at));
        Ok(envelopes)
    }

    /// Moves a message between mailboxes.
    ///
    /// JMAP patches `mailboxIds` by key rather than replacing the whole set, so
    /// two clients moving the same message into different folders do not clobber
    /// each other's other memberships. `true` adds, `null` removes.
    /// The identity to send a given address from.
    ///
    /// A JMAP account can hold several — aliases, a work address, a role
    /// account — and the server refuses a submission whose identity does not
    /// match the From it was given. Matching on the address rather than taking
    /// the first is what makes sending from an alias work.
    pub async fn identity_for(&self, address: &str) -> Result<String> {
        let backend_account = self.session.submission_account_id()?;
        let response = self
            .call_using(
                &[CAP_CORE, CAP_MAIL, CAP_SUBMISSION],
                vec![json!([
                    "Identity/get",
                    { "accountId": backend_account, "ids": Value::Null },
                    "id0"
                ])],
            )
            .await?;

        let list = response.args("id0")?.get("list").cloned().unwrap_or(json!([]));
        let identities: Vec<JmapIdentity> =
            serde_json::from_value(list).context("parsing identities")?;

        identities
            .iter()
            .find(|i| i.email.eq_ignore_ascii_case(address))
            .or_else(|| identities.first())
            .map(|i| i.id.clone())
            .ok_or_else(|| {
                anyhow!("this account has no identity able to send as {address}")
            })
    }

    /// Creates the message and submits it in one request.
    ///
    /// Both halves travel together on purpose. `EmailSubmission/set` refers to
    /// the draft by back-reference, so the email is never a real draft sitting
    /// in a mailbox waiting for a second round trip that might not happen —
    /// which is how clients leave a copy in Drafts *and* in Sent.
    ///
    /// `onSuccessUpdateEmail` files it in Sent and clears the draft keyword
    /// only if the submission actually succeeded, so a rejected send leaves
    /// the message where it can be found rather than filed as sent.
    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &self,
        from: &EmailAddress,
        identity_id: &str,
        drafts_mailbox: &str,
        sent_mailbox: &str,
        message: &crate::model::Outgoing,
    ) -> Result<()> {
        let backend_account = self.session.submission_account_id()?;

        let addresses = |list: &[EmailAddress]| -> Value {
            Value::Array(
                list.iter()
                    .map(|a| json!({ "name": a.name, "email": a.email }))
                    .collect(),
            )
        };

        let mut email = json!({
            "mailboxIds": { drafts_mailbox: true },
            "keywords": { "$draft": true },
            "from": [{ "name": from.name, "email": from.email }],
            "to": addresses(&message.to),
            "subject": message.subject,
            "bodyValues": { "text": { "value": message.text, "charset": "utf-8" } },
            "textBody": [{ "partId": "text", "type": "text/plain" }]
        });

        if !message.cc.is_empty() {
            email["cc"] = addresses(&message.cc);
        }
        if !message.bcc.is_empty() {
            email["bcc"] = addresses(&message.bcc);
        }
        if let Some(parent) = &message.in_reply_to {
            email["inReplyTo"] = json!([parent]);
        }
        if !message.references.is_empty() {
            email["references"] = json!(message.references);
        }

        let response = self
            .call_using(
                &[CAP_CORE, CAP_MAIL, CAP_SUBMISSION],
                vec![
                    json!([
                        "Email/set",
                        { "accountId": backend_account, "create": { "draft": email } },
                        "e0"
                    ]),
                    json!([
                        "EmailSubmission/set",
                        {
                            "accountId": backend_account,
                            "create": {
                                "send": { "emailId": "#draft", "identityId": identity_id }
                            },
                            "onSuccessUpdateEmail": {
                                "#send": {
                                    format!("mailboxIds/{drafts_mailbox}"): Value::Null,
                                    format!("mailboxIds/{sent_mailbox}"): true,
                                    "keywords/$draft": Value::Null
                                }
                            }
                        },
                        "s0"
                    ]),
                ],
            )
            .await?;

        // A JMAP method can succeed as a call and still refuse the object it was
        // given, so notCreated has to be read explicitly — otherwise a rejected
        // send looks exactly like a sent one.
        if let Some(failed) = response.args("e0")?.get("notCreated") {
            if !failed.is_null() && failed.as_object().is_some_and(|o| !o.is_empty()) {
                anyhow::bail!("the server would not accept the message: {failed}");
            }
        }
        if let Some(failed) = response.args("s0")?.get("notCreated") {
            if !failed.is_null() && failed.as_object().is_some_and(|o| !o.is_empty()) {
                anyhow::bail!("the server would not send the message: {failed}");
            }
        }
        Ok(())
    }

    pub async fn set_mailboxes(
        &self,
        email_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let backend_account = self.session.mail_account_id()?;

        let mut patch = serde_json::Map::new();
        for id in add {
            patch.insert(format!("mailboxIds/{id}"), json!(true));
        }
        for id in remove {
            patch.insert(format!("mailboxIds/{id}"), Value::Null);
        }

        let response = self
            .call(vec![json!([
                "Email/set",
                {
                    "accountId": backend_account,
                    "update": { email_id: Value::Object(patch) }
                },
                "s0"
            ])])
            .await?;

        // Email/set answers 200 even when it refuses an individual change, so the
        // rejection has to be dug out of notUpdated or it looks like a success.
        let args = response.args("s0")?;
        if let Some(rejected) = args.get("notUpdated").and_then(Value::as_object) {
            if let Some((id, reason)) = rejected.iter().next() {
                anyhow::bail!("JMAP refused to update {id}: {reason}");
            }
        }
        Ok(())
    }

    /// Sets or clears keywords — `$seen`, `$flagged` and friends.
    ///
    /// Patched by key like mailbox membership, so marking a message read cannot
    /// clobber a flag another client set at the same time.
    pub async fn set_keywords(
        &self,
        email_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let backend_account = self.session.mail_account_id()?;

        let mut patch = serde_json::Map::new();
        for keyword in add {
            patch.insert(format!("keywords/{keyword}"), json!(true));
        }
        for keyword in remove {
            patch.insert(format!("keywords/{keyword}"), Value::Null);
        }

        let response = self
            .call(vec![json!([
                "Email/set",
                {
                    "accountId": backend_account,
                    "update": { email_id: Value::Object(patch) }
                },
                "k0"
            ])])
            .await?;

        let args = response.args("k0")?;
        if let Some(rejected) = args.get("notUpdated").and_then(Value::as_object) {
            if let Some((id, reason)) = rejected.iter().next() {
                anyhow::bail!("JMAP refused to update {id}: {reason}");
            }
        }
        Ok(())
    }

    pub async fn body(&self, email_id: &str) -> Result<EmailBody> {
        let backend_account = self.session.mail_account_id()?;
        let response = self
            .call(vec![json!([
                "Email/get",
                {
                    "accountId": backend_account,
                    "ids": [email_id],
                    "properties": ["id", "htmlBody", "textBody", "bodyValues"],
                    "fetchHTMLBodyValues": true,
                    "fetchTextBodyValues": true,
                    "maxBodyValueBytes": 700_000
                },
                "b0"
            ])])
            .await?;

        let list = response.args("b0")?.get("list").cloned().unwrap_or(json!([]));
        let mut raw: Vec<JmapBody> = serde_json::from_value(list).context("parsing body")?;
        let body = raw
            .pop()
            .ok_or_else(|| anyhow!("no message returned for id {email_id}"))?;

        let pick = |parts: &[JmapBodyPart]| -> Option<String> {
            parts
                .iter()
                .filter_map(|p| p.part_id.as_ref())
                .find_map(|id| body.body_values.get(id).map(|v| v.value.clone()))
        };

        Ok(EmailBody {
            html: pick(&body.html_body),
            text: pick(&body.text_body),
            id: body.id,
        })
    }
}

// --- wire types -------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapMailbox {
    id: String,
    name: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    total_emails: u32,
    #[serde(default)]
    unread_emails: u32,
    #[serde(default)]
    sort_order: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapAddress {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapEmail {
    id: String,
    #[serde(default)]
    thread_id: String,
    #[serde(default)]
    mailbox_ids: HashMap<String, bool>,
    #[serde(default)]
    keywords: HashMap<String, bool>,
    #[serde(default)]
    from: Option<Vec<JmapAddress>>,
    #[serde(default)]
    to: Option<Vec<JmapAddress>>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    received_at: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    has_attachment: bool,
    #[serde(default)]
    message_id: Option<Vec<String>>,
    #[serde(default)]
    references: Option<Vec<String>>,
    #[serde(default, rename = "header:Authentication-Results:asText:all")]
    authentication_results: Option<Vec<String>>,
}

impl JmapEmail {
    fn into_envelope(self, account_id: &str) -> Envelope {
        let map = |v: Option<Vec<JmapAddress>>| -> Vec<EmailAddress> {
            v.unwrap_or_default()
                .into_iter()
                .map(|a| EmailAddress {
                    name: a.name,
                    email: a.email,
                })
                .collect()
        };
        Envelope {
            thread_id: if self.thread_id.is_empty() {
                self.id.clone()
            } else {
                self.thread_id
            },
            mailbox_ids: self.mailbox_ids.into_iter().map(|(k, _)| k).collect(),
            is_unread: !self.keywords.contains_key("$seen"),
            is_flagged: self.keywords.contains_key("$flagged"),
            from: map(self.from),
            to: map(self.to),
            subject: self.subject.unwrap_or_default(),
            preview: self.preview.unwrap_or_default(),
            received_at: self.received_at.unwrap_or_default(),
            has_attachment: self.has_attachment,
            // JMAP models both as lists because a malformed message can
            // carry several; the first is the one that identifies it.
            message_id: self.message_id.and_then(|ids| ids.into_iter().next()),
            references: self.references.unwrap_or_default(),
            // First, not last: headers are prepended as a message is handled,
            // so the topmost was added by our own provider.
            verified_domain: self
                .authentication_results
                .as_ref()
                .and_then(|all| all.first())
                .map(|header| crate::auth::parse(header))
                .and_then(|verdict| verdict.verified_domain().map(str::to_owned)),
            account_id: account_id.to_string(),
            id: self.id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapBody {
    id: String,
    #[serde(default)]
    html_body: Vec<JmapBodyPart>,
    #[serde(default)]
    text_body: Vec<JmapBodyPart>,
    #[serde(default)]
    body_values: HashMap<String, JmapBodyValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapBodyPart {
    #[serde(default)]
    part_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JmapBodyValue {
    #[serde(default)]
    value: String,
}
