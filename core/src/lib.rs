//! BazMail's sync engine.
//!
//! Everything that is not pixels lives here: the canonical model, the protocol
//! adapters, the local mirror and (later) the triage classifier and rules. It is
//! deliberately free of any UI or platform assumption so the same crate can be
//! hosted by Tauri today, exposed to Swift through UniFFI for the Apple apps,
//! and compiled to WASM for the web client.

pub mod at_rest;
pub mod auth;
pub mod config;
pub mod imap;
pub mod jmap;
pub mod model;
pub mod oauth;
pub mod secrets;
pub mod store;

pub use config::{account_id_from_address, AccountConfig, Config, ImapConfig};
pub use model::{
    Account, Connection, EmailAddress, EmailBody, Envelope, Lane, Mailbox, Mutation,
    Outgoing,
};

use anyhow::{anyhow, Result};
use imap::ImapClient;
use jmap::JmapClient;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use store::Store;

/// Which protocol an account speaks.
///
/// Everything above this point works in the canonical model and does not care
/// which arm it is talking to — that is the entire point of the adapters.
enum Backend {
    Jmap(Arc<JmapClient>),
    Imap(Arc<ImapClient>),
}

impl Backend {
    async fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        match self {
            Backend::Jmap(c) => c.mailboxes(account_id).await,
            Backend::Imap(c) => c.mailboxes(account_id).await,
        }
    }

    async fn envelopes(
        &self,
        account_id: &str,
        mailbox_id: &str,
        limit: usize,
    ) -> Result<Vec<Envelope>> {
        match self {
            Backend::Jmap(c) => c.envelopes(account_id, mailbox_id, limit).await,
            Backend::Imap(c) => c.envelopes(account_id, mailbox_id, limit).await,
        }
    }

    async fn body(&self, email_id: &str) -> Result<EmailBody> {
        match self {
            Backend::Jmap(c) => c.body(email_id).await,
            Backend::Imap(c) => c.body(email_id).await,
        }
    }

    /// JMAP patches a set of mailbox memberships; IMAP can only move a message
    /// to one place. The add list is therefore the destination, and anything
    /// beyond the first entry is meaningless on IMAP rather than silently
    /// half-applied.
    async fn set_flags(&self, email_id: &str, add: &[String], remove: &[String]) -> Result<()> {
        match self {
            Backend::Jmap(c) => c.set_keywords(email_id, add, remove).await,
            Backend::Imap(c) => c.set_flags(email_id, add, remove).await,
        }
    }

    /// Sends a message, filing it in Sent if the server accepts it.
    ///
    /// The identity lookup lives inside the JMAP arm rather than in the caller
    /// because it is a JMAP concept — an account there can hold several
    /// identities and the server rejects a submission whose identity does not
    /// match its From. SMTP has no equivalent to look up.
    async fn send(
        &self,
        from: &EmailAddress,
        drafts_mailbox: &str,
        sent_mailbox: &str,
        message: &Outgoing,
    ) -> Result<()> {
        match self {
            Backend::Jmap(c) => {
                let identity = c.identity_for(&from.email).await?;
                c.send(from, &identity, drafts_mailbox, sent_mailbox, message)
                    .await
            }
            // Reading and sending are different protocols on this path: IMAP
            // has no send verb at all, and the SMTP client that would provide
            // one does not exist yet. Saying so plainly beats failing somewhere
            // deeper with something that reads like a bug.
            Backend::Imap(_) => anyhow::bail!(
                "this account reads over IMAP, which cannot send — sending needs SMTP, \
                 and BazMail does not speak it yet"
            ),
        }
    }

    async fn set_mailboxes(
        &self,
        email_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        match self {
            Backend::Jmap(c) => c.set_mailboxes(email_id, add, remove).await,
            Backend::Imap(c) => {
                let destination = add
                    .first()
                    .ok_or_else(|| anyhow!("IMAP needs a destination mailbox to move a message"))?;
                c.move_message(email_id, destination).await?;
                Ok(())
            }
        }
    }
}

pub struct Engine {
    /// Mutable so signing in takes effect without a restart.
    config: RwLock<Config>,
    /// Held with a std mutex and never across an await — store calls are
    /// microseconds and a blocking lock is honest about that.
    store: Mutex<Store>,
    http: reqwest::Client,
    /// Cached alongside the token it was built with, so a refresh naturally
    /// invalidates the client instead of leaving it authenticating with a token
    /// that has expired.
    clients: tokio::sync::Mutex<HashMap<String, (String, Arc<Backend>)>>,
    /// Access tokens are short-lived; this avoids a refresh round trip on every
    /// single call. Keyed by account id, valid until the instant stored with it.
    access_tokens: tokio::sync::Mutex<HashMap<String, (String, Instant)>>,
    /// Stored credentials, read from the OS store once per launch.
    ///
    /// Not an optimisation — a correctness fix on macOS. The Keychain asks the
    /// user to authorise every read by an unsigned application, and Fastmail
    /// rotates refresh tokens, so each refresh wrote a new credential and the
    /// following read prompted again. The result was an endless run of password
    /// dialogs that entering the password could not end, because each one was a
    /// fresh question rather than a rejected answer.
    ///
    /// The exposure this adds is small: the same secret is already held in
    /// memory for the duration of every call that uses it, and the access-token
    /// cache above already makes exactly this trade.
    credentials: tokio::sync::Mutex<HashMap<String, String>>,
}

/// What a sync actually did, so the UI can say something truthful rather than
/// flashing a spinner and hoping.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub account_id: String,
    pub mailboxes: usize,
    pub envelopes: usize,
}

impl Engine {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let db_path = Config::config_dir()?.join("bazmail.db");
        let store = Store::open(&db_path)?;
        Ok(Self {
            config: RwLock::new(config),
            store: Mutex::new(store),
            http: reqwest::Client::builder()
                .user_agent(concat!("BazMail/", env!("CARGO_PKG_VERSION")))
                .build()?,
            clients: tokio::sync::Mutex::new(HashMap::new()),
            access_tokens: tokio::sync::Mutex::new(HashMap::new()),
            credentials: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn is_configured(&self) -> bool {
        !self.config.read().unwrap().is_empty()
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.config
            .read()
            .unwrap()
            .accounts
            .iter()
            .map(|a| Account {
                id: a.id.clone(),
                label: a.label.clone(),
                color: a.color.clone(),
                identity: a.identity.clone(),
                connection: if a.imap.is_some() {
                    Connection::Imap
                } else if a.client_id.is_some() {
                    Connection::Oauth
                } else {
                    Connection::Token
                },
            })
            .collect()
    }

    /// Moves an account one place up or down the sidebar.
    ///
    /// Order lives in the config array itself rather than in a separate rank
    /// column: there is exactly one list, it is short, and a swap of adjacent
    /// entries cannot produce the duplicate or missing ranks that a numeric
    /// ordering eventually does.
    pub fn move_account(&self, account_id: &str, up: bool) -> Result<()> {
        self.mutate_config(|config| {
            let Some(at) = config.accounts.iter().position(|a| a.id == account_id) else {
                return;
            };
            let to = if up {
                at.checked_sub(1)
            } else {
                Some(at + 1).filter(|i| *i < config.accounts.len())
            };
            // At either end there is nowhere to go, and that is not an error —
            // the menu item is simply disabled.
            if let Some(to) = to {
                config.accounts.swap(at, to);
            }
        })
    }

    /// Repaints an account. Provenance colour is the only way to tell two
    /// accounts apart at a glance in a unified list, so a collision makes the
    /// unified inbox unreadable — and until now nothing could fix one.
    pub fn set_account_color(&self, account_id: &str, color: &str) -> Result<()> {
        self.mutate_config(|config| {
            if let Some(account) = config.accounts.iter_mut().find(|a| a.id == account_id) {
                account.color = color.to_string();
            }
        })
    }

    /// Changes the config and persists it, re-reading from disk first.
    ///
    /// The in-memory copy is loaded once at startup, so writing it back wholesale
    /// discards anything another instance has added since. That is not
    /// theoretical: it silently dropped an OAuth `client_id` and an entire iCloud
    /// account, leaving credentials orphaned in the OS store while the app
    /// insisted the accounts did not exist. Re-reading makes a write a merge.
    fn mutate_config(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut held = self.config.write().unwrap();
        let mut fresh = Config::load()?;
        change(&mut fresh);
        fresh.save()?;
        *held = fresh;
        Ok(())
    }

    fn account_config(&self, account_id: &str) -> Result<AccountConfig> {
        self.config
            .read()
            .unwrap()
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .cloned()
            .ok_or_else(|| anyhow!("no account configured with id '{account_id}'"))
    }

    /// The bearer token to use right now.
    ///
    /// For an API-token account this is just the stored credential. For an OAuth
    /// account the stored credential is a *refresh* token, and this trades it for
    /// a short-lived access token, caching that until shortly before it expires.
    /// Reads a stored credential, holding it for the life of the process.
    ///
    /// See the `credentials` field for why this is not merely a speed-up.
    async fn credential(&self, account: &AccountConfig) -> Result<String> {
        if let Some(cached) = self.credentials.lock().await.get(&account.id) {
            return Ok(cached.clone());
        }
        let token = account.resolve_token()?;
        self.credentials
            .lock()
            .await
            .insert(account.id.clone(), token.clone());
        Ok(token)
    }

    /// Records a credential we just wrote, so the next use does not go back to
    /// the OS store and ask the user about it again.
    async fn remember_credential(&self, account_id: &str, token: &str) {
        self.credentials
            .lock()
            .await
            .insert(account_id.to_string(), token.to_string());
    }

    /// Drops a cached credential. Called when an account is removed, so a
    /// re-added account never authenticates with the previous one's token.
    async fn forget_credential(&self, account_id: &str) {
        self.credentials.lock().await.remove(account_id);
    }

    async fn access_token(&self, account: &AccountConfig) -> Result<String> {
        if !account.is_oauth() {
            return self.credential(account).await;
        }

        if let Some((token, expires)) = self.access_tokens.lock().await.get(&account.id) {
            if Instant::now() < *expires {
                return Ok(token.clone());
            }
        }

        let client_id = account
            .client_id
            .as_deref()
            .ok_or_else(|| anyhow!("account '{}' has no OAuth client id", account.id))?;
        let refresh_token = self.credential(account).await?;

        let endpoints = oauth::discover(&self.http, oauth::FASTMAIL_ISSUER).await?;
        let tokens = oauth::refresh(&self.http, &endpoints, client_id, &refresh_token).await?;

        // Refresh tokens can rotate; storing the new one is what stops the next
        // refresh failing with invalid_grant.
        if let Some(rotated) = tokens.refresh_token.as_deref() {
            if rotated != refresh_token {
                secrets::store_token(&account.id, rotated)?;
                self.remember_credential(&account.id, rotated).await;
            }
        }

        // Expire a minute early so a call never goes out on a token about to die.
        let lifetime = Duration::from_secs(tokens.expires_in.unwrap_or(3600).saturating_sub(60).max(30));
        self.access_tokens.lock().await.insert(
            account.id.clone(),
            (tokens.access_token.clone(), Instant::now() + lifetime),
        );
        Ok(tokens.access_token)
    }

    /// Connects on first use and keeps the session; JMAP sessions are cheap to
    /// hold and re-fetching one per request would double every round trip.
    async fn client(&self, account_id: &str) -> Result<Arc<Backend>> {
        let account = self.account_config(account_id)?;
        let credential = self.access_token(&account).await?;

        if let Some((cached, client)) = self.clients.lock().await.get(account_id) {
            if cached == &credential {
                return Ok(client.clone());
            }
        }

        let backend = if let Some(imap) = account.imap.as_ref() {
            Arc::new(Backend::Imap(Arc::new(ImapClient::new(
                imap.host.clone(),
                imap.port,
                imap.username.clone(),
                credential.clone(),
            ))))
        } else {
            Arc::new(Backend::Jmap(Arc::new(
                JmapClient::connect(self.http.clone(), &account.session_url, credential.clone())
                    .await?,
            )))
        };

        self.clients
            .lock()
            .await
            .insert(account_id.to_string(), (credential, backend.clone()));
        Ok(backend)
    }

    pub async fn sync_account(&self, account_id: &str, limit: usize) -> Result<SyncReport> {
        let client = self.client(account_id).await?;

        let mailboxes = client.mailboxes(account_id).await?;
        {
            let store = self.store.lock().unwrap();
            store.put_mailboxes(&mailboxes)?;
        }

        let inbox = mailboxes
            .iter()
            .find(|m| m.role.as_deref() == Some("inbox"))
            .ok_or_else(|| anyhow!("account '{account_id}' exposes no inbox"))?;

        let envelopes = client.envelopes(account_id, &inbox.id, limit).await?;
        {
            let store = self.store.lock().unwrap();
            store.put_envelopes(&envelopes)?;
        }

        Ok(SyncReport {
            account_id: account_id.to_string(),
            mailboxes: mailboxes.len(),
            envelopes: envelopes.len(),
        })
    }

    /// Fetches one mailbox on demand.
    ///
    /// Startup sync only pulls inboxes — pulling every folder of every account
    /// would be slow and mostly wasted. So anything else is fetched when you
    /// first open it, which is why Spam and Archive were empty until now.
    pub async fn sync_mailbox(
        &self,
        account_id: &str,
        mailbox_id: &str,
        limit: usize,
    ) -> Result<usize> {
        let client = self.client(account_id).await?;
        let envelopes = client.envelopes(account_id, mailbox_id, limit).await?;
        let count = envelopes.len();
        self.store.lock().unwrap().put_envelopes(&envelopes)?;
        Ok(count)
    }

    /// Syncs every configured account. One account failing does not stop the
    /// others — a dead token on personal mail must not take work mail down.
    pub async fn sync_all(&self, limit: usize) -> Vec<Result<SyncReport>> {
        // Snapshot the ids so the lock is not held across an await.
        let ids: Vec<String> = self
            .config
            .read()
            .unwrap()
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let mut reports = Vec::new();
        for id in ids {
            reports.push(self.sync_account(&id, limit).await);
        }
        reports
    }

    pub fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        self.store.lock().unwrap().mailboxes(account_id)
    }

    /// The unified inbox: every account's inbox, merged, newest first.
    pub fn unified_inbox(&self, limit: usize) -> Result<Vec<Envelope>> {
        let ids: Vec<String> = self
            .config
            .read()
            .unwrap()
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let store = self.store.lock().unwrap();
        let mut inbox_ids = Vec::new();
        for id in &ids {
            for mailbox in store.mailboxes(id)? {
                if mailbox.role.as_deref() == Some("inbox") {
                    inbox_ids.push(mailbox.id);
                }
            }
        }
        if inbox_ids.is_empty() {
            return Ok(Vec::new());
        }
        store.envelopes(&inbox_ids, limit)
    }

    /// Sends a message and files the result in Sent.
    ///
    /// Not queued through the outbox, deliberately. Everything else there is
    /// idempotent — archiving twice archives once — but sending twice sends
    /// twice, and a retry loop that duplicates mail is worse than one that
    /// fails visibly. A proper outbox for sending needs a deduplication key the
    /// server honours, which is its own piece of work.
    pub async fn send(&self, message: &Outgoing) -> Result<()> {
        let identity = self
            .config
            .read()
            .unwrap()
            .accounts
            .iter()
            .find(|a| a.id == message.account_id)
            .map(|a| a.identity.clone())
            .ok_or_else(|| anyhow!("no account called '{}'", message.account_id))?;

        let from = EmailAddress {
            name: None,
            email: identity,
        };
        let drafts = self.mailbox_by_role(&message.account_id, "drafts")?;
        let sent = self.mailbox_by_role(&message.account_id, "sent")?;

        self.client(&message.account_id)
            .await?
            .send(&from, &drafts, &sent, message)
            .await?;

        // Pull Sent straight away so the message you just sent is visible
        // rather than absent until whenever the next sync happens.
        let _ = self.sync_mailbox(&message.account_id, &sent, 50).await;
        Ok(())
    }

    /// Everything the mirror holds for one mailbox, newest first.
    ///
    /// Separate from `unified_inbox` because that one deliberately narrows to
    /// inbox-role mailboxes. Reading a specific mailbox through it and then
    /// filtering — which is what the UI was doing — can only ever return
    /// nothing for Spam, Archive or any other folder, since none of their
    /// messages are in the set it returns in the first place.
    pub fn mailbox_envelopes(&self, mailbox_id: &str, limit: usize) -> Result<Vec<Envelope>> {
        self.store
            .lock()
            .unwrap()
            .envelopes(&[mailbox_id.to_string()], limit)
    }

    /// Bodies are fetched on demand and not yet cached — caching them is worth
    /// doing, but only alongside the sanitiser, since an unsanitised body in the
    /// store is a liability we would then have to migrate.
    pub async fn body(&self, account_id: &str, email_id: &str) -> Result<EmailBody> {
        self.client(account_id).await?.body(email_id).await
    }

    /// Looks up a mailbox id by its JMAP role for one account.
    fn mailbox_by_role(&self, account_id: &str, role: &str) -> Result<String> {
        let store = self.store.lock().unwrap();
        store
            .mailboxes(account_id)?
            .into_iter()
            .find(|m| m.role.as_deref() == Some(role))
            .map(|m| m.id)
            .ok_or_else(|| anyhow!("account '{account_id}' has no {role} mailbox"))
    }

    /// Archives a message: out of the inbox, into the archive.
    ///
    /// The mirror is updated and the change queued before anything touches the
    /// network, so the row leaves the list on the same frame as the keypress. The
    /// send is then attempted; if it fails the change stays queued rather than
    /// being lost, and the UI is not rolled back — the local state is what the
    /// user asked for, and sync will reconcile it.
    pub async fn archive(&self, account_id: &str, email_id: &str) -> Result<ArchiveOutcome> {
        let inbox = self.mailbox_by_role(account_id, "inbox")?;
        let archive = self.mailbox_by_role(account_id, "archive")?;
        self.move_message(account_id, email_id, "archive", &[archive], &[inbox])
            .await
    }

    /// The inverse. Undo re-issues the opposite move rather than cancelling the
    /// queued one, so it behaves the same whether or not the archive already
    /// reached the server.
    pub async fn unarchive(&self, account_id: &str, email_id: &str) -> Result<ArchiveOutcome> {
        let inbox = self.mailbox_by_role(account_id, "inbox")?;
        let archive = self.mailbox_by_role(account_id, "archive")?;
        self.move_message(account_id, email_id, "unarchive", &[inbox], &[archive])
            .await
    }

    async fn move_message(
        &self,
        account_id: &str,
        email_id: &str,
        kind: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<ArchiveOutcome> {
        let mutation_id = {
            let store = self.store.lock().unwrap();
            store.apply_mailbox_change(account_id, email_id, add, remove)?;
            store.enqueue(account_id, email_id, kind, add, remove)?
        };

        let synced = self.flush_outbox().await.unwrap_or(0);
        let queued = self.store.lock().unwrap().pending_count()?;

        Ok(ArchiveOutcome {
            mutation_id,
            account_id: account_id.to_string(),
            email_id: email_id.to_string(),
            synced,
            queued,
        })
    }

    /// Drains the queue oldest-first. A mutation that fails keeps its place with
    /// the error recorded, and blocks the ones behind it for the same message —
    /// order matters, since an archive and its undo must not arrive reversed.
    pub async fn flush_outbox(&self) -> Result<usize> {
        let pending = self.store.lock().unwrap().pending(64)?;
        let mut sent = 0;

        for mutation in pending {
            let client = match self.client(&mutation.account_id).await {
                Ok(c) => c,
                Err(e) => {
                    self.store
                        .lock()
                        .unwrap()
                        .record_failure(mutation.id, &e.to_string())?;
                    continue;
                }
            };

            // A mutation is either a move or a flag change, never both, so the
            // flag lists are what distinguishes them.
            let sent_result = if mutation.add_flags.is_empty() && mutation.remove_flags.is_empty() {
                client
                    .set_mailboxes(
                        &mutation.email_id,
                        &mutation.add_mailbox_ids,
                        &mutation.remove_mailbox_ids,
                    )
                    .await
            } else {
                client
                    .set_flags(&mutation.email_id, &mutation.add_flags, &mutation.remove_flags)
                    .await
            };

            match sent_result {
                Ok(()) => {
                    self.store.lock().unwrap().drop_mutation(mutation.id)?;
                    sent += 1;
                }
                Err(e) => {
                    self.store
                        .lock()
                        .unwrap()
                        .record_failure(mutation.id, &e.to_string())?;
                }
            }
        }
        Ok(sent)
    }

    pub fn pending_mutations(&self) -> Result<usize> {
        self.store.lock().unwrap().pending_count()
    }

    /// Starts an OAuth sign-in and returns the URL to send the user to.
    ///
    /// The loopback listener is bound *before* the URL is handed back, so the
    /// redirect can never arrive at a closed port. Split from `finish_oauth` so
    /// that opening a browser — a platform concern — stays out of the engine.
    pub async fn start_oauth(&self) -> Result<PendingAuth> {
        let endpoints = oauth::discover(&self.http, oauth::FASTMAIL_ISSUER).await?;

        // Port 0 lets the OS pick a free one; RFC 8252 requires the authorization
        // server to accept any port on a loopback redirect.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://localhost:{port}/callback");

        let client_id = oauth::register(&self.http, &endpoints, &redirect_uri).await?;
        let pkce = oauth::pkce();
        let state = oauth::random_urlsafe(16);
        let authorize_url =
            oauth::authorize_url(&endpoints, &client_id, &redirect_uri, &state, &pkce.challenge);

        Ok(PendingAuth {
            authorize_url,
            endpoints,
            client_id,
            redirect_uri,
            verifier: pkce.verifier,
            state,
            listener,
        })
    }

    /// Waits for the redirect, exchanges the code, and saves the account.
    ///
    /// The refresh token is what gets stored — access tokens are short-lived and
    /// deliberately never written to disk.
    pub async fn finish_oauth(&self, pending: PendingAuth, color: String) -> Result<Account> {
        let code = oauth::wait_for_code(
            pending.listener,
            &pending.state,
            std::time::Duration::from_secs(300),
        )
        .await?;

        let tokens = oauth::exchange_code(
            &self.http,
            &pending.endpoints,
            &pending.client_id,
            &pending.redirect_uri,
            &code,
            &pending.verifier,
        )
        .await?;

        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow!("Fastmail returned no refresh token — was offline_access granted?"))?;

        // Confirm the grant actually works before writing anything, so a saved
        // account is always a working one.
        let session_url = config::default_session_url();
        let client = JmapClient::connect(
            self.http.clone(),
            &session_url,
            tokens.access_token.clone(),
        )
        .await?;
        let session = client.session();
        session.mail_account_id()?;

        let username = session.username.clone();
        let id = account_id_from_address(&username);
        let label = username
            .split('@')
            .nth(1)
            .filter(|d| !d.is_empty())
            .unwrap_or(&username)
            .to_string();

        secrets::store_token(&id, &refresh_token)?;
        self.remember_credential(&id, &refresh_token).await;

        let account = Account {
            id: id.clone(),
            label: label.clone(),
            color: color.clone(),
            identity: username.clone(),
            connection: Connection::Oauth,
        };

        self.mutate_config(|config| {
            config.upsert(AccountConfig {
                id: id.clone(),
                label,
                color,
                identity: username,
                session_url,
                token: None,
                token_env: None,
                client_id: Some(pending.client_id),
                imap: None,
            })
        })?;

        self.clients.lock().await.remove(&id);
        self.access_tokens.lock().await.remove(&id);
        Ok(account)
    }

    /// Marks a message read or unread.
    ///
    /// Same optimistic path as archiving: the mirror changes first and the
    /// change is queued, so the unread dot clears immediately and a flaky
    /// network cannot make the UI lie about what you have read.
    pub async fn mark_read(&self, account_id: &str, email_id: &str, read: bool) -> Result<()> {
        let seen = vec!["$seen".to_string()];
        let (add, remove) = if read {
            (seen, Vec::new())
        } else {
            (Vec::new(), seen)
        };

        {
            let store = self.store.lock().unwrap();
            store.set_unread(account_id, email_id, !read)?;
            store.enqueue_full(
                account_id,
                email_id,
                if read { "mark-read" } else { "mark-unread" },
                &[],
                &[],
                &add,
                &remove,
            )?;
        }

        // Best effort: it is already recorded locally and queued, so a failure
        // here is a retry rather than an error worth surfacing.
        let _ = self.flush_outbox().await;
        Ok(())
    }

    /// Connects an IMAP account — iCloud, and anything else without a real API.
    ///
    /// Verified by actually logging in and listing mailboxes before anything is
    /// written, so a saved account is always a working one. The app-specific
    /// password goes to the OS credential store; only the host, port and
    /// username reach config.json.
    pub async fn connect_imap(
        &self,
        host: String,
        port: u16,
        username: String,
        password: String,
        label: String,
        color: String,
    ) -> Result<Account> {
        let id = account_id_from_address(&username);
        let client = ImapClient::new(host.clone(), port, username.clone(), password.clone());

        let mailboxes = client.mailboxes(&id).await?;
        if !mailboxes.iter().any(|m| m.role.as_deref() == Some("inbox")) {
            anyhow::bail!("connected, but the server exposed no inbox");
        }

        secrets::store_token(&id, &password)?;
        self.remember_credential(&id, &password).await;

        let label = if label.trim().is_empty() {
            username
                .split('@')
                .nth(1)
                .filter(|d| !d.is_empty())
                .unwrap_or(&username)
                .to_string()
        } else {
            label
        };

        let account = Account {
            id: id.clone(),
            label: label.clone(),
            color: color.clone(),
            identity: username.clone(),
            connection: Connection::Imap,
        };

        self.mutate_config(|config| {
            config.upsert(AccountConfig {
                id: id.clone(),
                label,
                color,
                identity: username.clone(),
                session_url: String::new(),
                token: None,
                token_env: None,
                client_id: None,
                imap: Some(ImapConfig {
                    host,
                    port,
                    username,
                }),
            })
        })?;

        self.clients.lock().await.remove(&id);
        Ok(account)
    }

    /// Checks a token by actually fetching a JMAP session with it, and reports
    /// back who it belongs to. Nothing is written — this is the step that lets
    /// the UI say "connected as X" before anything is persisted, so a typo never
    /// becomes a saved-but-broken account.
    pub async fn verify_token(&self, session_url: &str, token: &str) -> Result<VerifiedAccount> {
        let client =
            JmapClient::connect(self.http.clone(), session_url, token.to_string()).await?;
        let session = client.session();
        // Confirms the token actually carries mail scope, not just that it authenticated.
        session.mail_account_id()?;

        let username = session.username.clone();
        let account_name = session
            .primary_accounts
            .get("urn:ietf:params:jmap:mail")
            .and_then(|id| session.accounts.get(id))
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let domain = username.split('@').nth(1).unwrap_or("").to_string();

        Ok(VerifiedAccount {
            suggested_id: account_id_from_address(&username),
            suggested_label: if domain.is_empty() {
                username.clone()
            } else {
                domain
            },
            username,
            account_name,
        })
    }

    /// Verifies before persisting, so a saved account is always a working one.
    /// The token goes to the OS credential store; only non-secret metadata is
    /// written to config.json.
    pub async fn save_account(
        &self,
        id: String,
        label: String,
        color: String,
        identity: String,
        session_url: String,
        token: String,
    ) -> Result<()> {
        self.verify_token(&session_url, &token).await?;
        secrets::store_token(&id, &token)?;
        self.remember_credential(&id, &token).await;

        self.mutate_config(|config| {
            config.upsert(AccountConfig {
                id: id.clone(),
                label,
                color,
                identity,
                session_url,
                token: None,
                token_env: None,
                client_id: None,
                imap: None,
            })
        })?;

        // Drop any cached session so the next call picks up the new credential.
        self.clients.lock().await.remove(&id);
        Ok(())
    }

    pub async fn delete_account(&self, account_id: &str) -> Result<()> {
        self.mutate_config(|config| config.remove(account_id))?;
        secrets::delete_token(account_id)?;
        self.clients.lock().await.remove(account_id);
        self.access_tokens.lock().await.remove(account_id);
        // Otherwise an account re-added under the same id would authenticate
        // with the removed one's token — which is exactly the case reconnecting
        // hits, since ids are derived from the address.
        self.forget_credential(account_id).await;
        Ok(())
    }
}

/// A sign-in in flight: the URL to open, and everything needed to finish once
/// the redirect comes back. Owns the listener so the port stays bound.
pub struct PendingAuth {
    pub authorize_url: String,
    endpoints: oauth::Endpoints,
    client_id: String,
    redirect_uri: String,
    verifier: String,
    state: String,
    listener: tokio::net::TcpListener,
}

/// The result of a move, so the UI can say whether it actually reached the
/// server or is sitting in the queue.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOutcome {
    pub mutation_id: i64,
    pub account_id: String,
    pub email_id: String,
    pub synced: usize,
    pub queued: usize,
}

/// What a token turned out to belong to, so the UI can confirm before saving.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedAccount {
    pub username: String,
    pub account_name: String,
    pub suggested_id: String,
    pub suggested_label: String,
}
