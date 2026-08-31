//! Diagnostic: prints exactly what the engine sees at startup.
//!
//! Run with `cargo run -p bazmail-core --example config_check`. Useful when the
//! app disagrees with what is plainly on disk — it separates "the file is wrong"
//! from "we are reading it wrong" without guessing.

use bazmail_core::{secrets, store::Store, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = Config::config_path()?;
    println!("config path : {}", path.display());
    println!("exists      : {}", path.exists());

    let config = Config::load()?;
    println!("accounts    : {}", config.accounts.len());
    println!("is_empty    : {}", config.is_empty());

    for account in &config.accounts {
        println!("\n  id        : {}", account.id);
        println!("  label     : {}", account.label);
        println!("  identity  : {}", account.identity);
        println!("  sessionUrl: {}", account.session_url);
        println!("  inline tok: {}", account.token.is_some());
        println!("  tokenEnv  : {:?}", account.token_env);

        // Presence only — never the value.
        match secrets::load_token(&account.id) {
            Ok(Some(t)) => println!("  credential: present ({} chars)", t.len()),
            Ok(None) => println!("  credential: MISSING from the OS store"),
            Err(e) => println!("  credential: ERROR {e}"),
        }
    }

    // Write the loaded config straight back. Same content, so it is harmless,
    // and it proves whether the real path is writable at all.
    match config.save() {
        Ok(()) => println!("save        : ok"),
        Err(e) => println!("save        : FAILED — {e:#}"),
    }
    println!(
        "mtime       : {:?}",
        std::fs::metadata(&path).and_then(|m| m.modified()).ok()
    );

    // Try each account for real — the point is to separate "the credential is
    // missing" from "the credential is there and the server refuses it".
    let engine = bazmail_core::Engine::new()?;
    for account in &config.accounts {
        println!("
connecting  : {}", account.id);
        match engine.mailboxes(&account.id) {
            Ok(m) => println!("  cached    : {} mailboxes", m.len()),
            Err(e) => println!("  cached    : {e}"),
        }
        match engine.sync_account(&account.id, 5).await {
            Ok(r) => println!("  live      : ok, {} mailboxes, {} messages", r.mailboxes, r.envelopes),
            Err(e) => println!("  live      : FAILED — {e:#}"),
        }
    }

    // Opening the store is what applies at-rest protection, so this doubles as
    // the way to trigger it outside the app.
    let db = Config::config_dir()?.join("bazmail.db");
    println!("
store       : {}", db.display());
    println!("plaintext   : {}", bazmail_core::at_rest::is_plaintext_sqlite(&db));
    Store::open(&db)?;
    println!("protection  : {:?}", bazmail_core::at_rest::protection(&db));
    Ok(())
}
