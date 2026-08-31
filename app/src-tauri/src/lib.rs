//! The Tauri host.
//!
//! Thin by design: it owns a window and a command surface, and nothing else.
//! All the behaviour lives in `bazmail-core` so the Apple and web clients can
//! host the identical engine without reimplementing anything.

use bazmail_core::{
    Account, ArchiveOutcome, Config, EmailBody, Engine, Envelope, Mailbox, VerifiedAccount,
};
use tauri_plugin_opener::OpenerExt;
use serde::Serialize;
use tauri::{Manager, State};

/// Commands hand errors back as strings — the frontend shows them, it does not
/// branch on them, and an error enum would be ceremony for no gain today.
type CmdResult<T> = Result<T, String>;

fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncOutcome {
    account_id: String,
    ok: bool,
    envelopes: usize,
    mailboxes: usize,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    configured: bool,
    config_path: String,
    accounts: Vec<Account>,
    /// Everything the settings panel needs to tell the truth about where data
    /// lives and how it is protected, rather than claiming either.
    store_path: String,
    store_protection: bazmail_core::at_rest::Protection,
    version: String,
    queued_mutations: usize,
}

#[tauri::command]
fn status(engine: State<'_, Engine>) -> CmdResult<Status> {
    let store_path = Config::config_dir()
        .map(|d| d.join("bazmail.db"))
        .unwrap_or_default();

    Ok(Status {
        configured: engine.is_configured(),
        config_path: Config::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        accounts: engine.accounts(),
        store_protection: bazmail_core::at_rest::protection(&store_path),
        store_path: store_path.display().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        queued_mutations: engine.pending_mutations().unwrap_or(0),
    })
}

#[tauri::command]
async fn sync_all(engine: State<'_, Engine>, limit: usize) -> CmdResult<Vec<SyncOutcome>> {
    let reports = engine.sync_all(limit).await;
    let accounts = engine.accounts();

    Ok(reports
        .into_iter()
        .zip(accounts)
        .map(|(report, account)| match report {
            Ok(r) => SyncOutcome {
                account_id: r.account_id,
                ok: true,
                envelopes: r.envelopes,
                mailboxes: r.mailboxes,
                error: None,
            },
            Err(e) => SyncOutcome {
                account_id: account.id,
                ok: false,
                envelopes: 0,
                mailboxes: 0,
                error: Some(e.to_string()),
            },
        })
        .collect())
}

#[tauri::command]
async fn sync_mailbox(
    engine: State<'_, Engine>,
    account_id: String,
    mailbox_id: String,
    limit: usize,
) -> CmdResult<usize> {
    engine
        .sync_mailbox(&account_id, &mailbox_id, limit)
        .await
        .map_err(fail)
}

#[tauri::command]
fn unified_inbox(engine: State<'_, Engine>, limit: usize) -> CmdResult<Vec<Envelope>> {
    engine.unified_inbox(limit).map_err(fail)
}

#[tauri::command]
fn mailboxes(engine: State<'_, Engine>, account_id: String) -> CmdResult<Vec<Mailbox>> {
    engine.mailboxes(&account_id).map_err(fail)
}

#[tauri::command]
async fn body(
    engine: State<'_, Engine>,
    account_id: String,
    email_id: String,
) -> CmdResult<EmailBody> {
    engine.body(&account_id, &email_id).await.map_err(fail)
}

/// Checks a token without saving anything, so the UI can confirm who it belongs
/// to before committing. The token is passed through and never logged.
#[tauri::command]
async fn verify_token(
    engine: State<'_, Engine>,
    session_url: String,
    token: String,
) -> CmdResult<VerifiedAccount> {
    engine.verify_token(&session_url, &token).await.map_err(fail)
}

/// Persists an account. The token goes to the OS credential store; config.json
/// only ever sees the non-secret metadata.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn save_account(
    engine: State<'_, Engine>,
    id: String,
    label: String,
    color: String,
    identity: String,
    session_url: String,
    token: String,
) -> CmdResult<()> {
    engine
        .save_account(id, label, color, identity, session_url, token)
        .await
        .map_err(fail)
}

#[tauri::command]
async fn delete_account(engine: State<'_, Engine>, account_id: String) -> CmdResult<()> {
    engine.delete_account(&account_id).await.map_err(fail)
}

/// The whole OAuth sign-in, start to finish.
///
/// The engine binds the loopback listener and hands back a URL; opening a
/// browser is a platform job, so that part happens here. Then it blocks until
/// the redirect arrives or five minutes pass.
#[tauri::command]
async fn connect_fastmail(
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    color: String,
) -> CmdResult<Account> {
    let pending = engine.start_oauth().await.map_err(fail)?;

    // The system browser, deliberately — not a webview we control. The user is
    // typing a password, and it should be somewhere they can see the real URL
    // and their own password manager can reach.
    app.opener()
        .open_url(pending.authorize_url.clone(), None::<&str>)
        .map_err(fail)?;

    engine.finish_oauth(pending, color).await.map_err(fail)
}

/// Connects an IMAP account. iCloud is the case this exists for: Apple offers
/// no OAuth for third-party mail, so an app-specific password is the only way in.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn connect_imap(
    engine: State<'_, Engine>,
    host: String,
    port: u16,
    username: String,
    password: String,
    label: String,
    color: String,
) -> CmdResult<Account> {
    engine
        .connect_imap(host, port, username, password, label, color)
        .await
        .map_err(fail)
}

#[tauri::command]
fn move_account(engine: State<'_, Engine>, account_id: String, up: bool) -> CmdResult<()> {
    engine.move_account(&account_id, up).map_err(fail)
}

#[tauri::command]
fn set_account_color(
    engine: State<'_, Engine>,
    account_id: String,
    color: String,
) -> CmdResult<()> {
    engine.set_account_color(&account_id, &color).map_err(fail)
}

#[tauri::command]
async fn mark_read(
    engine: State<'_, Engine>,
    account_id: String,
    email_id: String,
    read: bool,
) -> CmdResult<()> {
    engine.mark_read(&account_id, &email_id, read).await.map_err(fail)
}

#[tauri::command]
async fn archive(
    engine: State<'_, Engine>,
    account_id: String,
    email_id: String,
) -> CmdResult<ArchiveOutcome> {
    engine.archive(&account_id, &email_id).await.map_err(fail)
}

#[tauri::command]
async fn unarchive(
    engine: State<'_, Engine>,
    account_id: String,
    email_id: String,
) -> CmdResult<ArchiveOutcome> {
    engine.unarchive(&account_id, &email_id).await.map_err(fail)
}

/// Retries whatever is still queued. Called on startup so changes made while
/// offline go out as soon as there is a network again.
#[tauri::command]
async fn flush_outbox(engine: State<'_, Engine>) -> CmdResult<usize> {
    engine.flush_outbox().await.map_err(fail)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // A failure here means the store could not be opened at all, which is
            // unrecoverable — better to die loudly than to run with no mirror.
            // Desktop only: there is no updater on mobile, where the store
            // owns installation.
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            let engine = Engine::new()?;
            app.manage(engine);

            // The window is built here rather than by the config so it can carry
            // a new-window handler. Links inside a message body target _blank,
            // so every click a sender authored arrives at this closure — and
            // none of them is allowed to become a window.
            //
            // They go to the real browser instead, which is the point: the
            // address bar is visible, the user's extensions and password manager
            // are there, and no HTML written by a stranger ever renders in a
            // webview of ours.
            let handle = app.handle().clone();
            let config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("no window is configured")?;

            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                .on_new_window(move |url, _features| {
                    // Only the two web schemes are handed to the OS. A sender
                    // can write file:, javascript: or anything else into an
                    // href, and passing those to the shell would be handing a
                    // stranger the ability to open local files.
                    if matches!(url.scheme(), "http" | "https") {
                        if let Err(e) = handle.opener().open_url(url.to_string(), None::<&str>) {
                            eprintln!("could not open {url} in the browser: {e}");
                        }
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            sync_all,
            sync_mailbox,
            unified_inbox,
            mailboxes,
            body,
            archive,
            set_account_color,
            move_account,
            mark_read,
            unarchive,
            flush_outbox,
            connect_fastmail,
            connect_imap,
            verify_token,
            save_account,
            delete_account
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
