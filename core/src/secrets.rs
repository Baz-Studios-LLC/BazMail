//! Credential storage.
//!
//! Tokens go to the OS credential store — Windows Credential Manager here, the
//! Keychain on macOS and iOS — not to a file next to the mail database. A
//! plaintext token on disk was an acceptable bootstrap; it is not an acceptable
//! place to leave a credential that can read every message in an account.
//!
//! Nothing in this module logs a token, and no error it returns contains one.

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "BazMail";

fn entry(account_id: &str) -> Result<Entry> {
    Entry::new(SERVICE, account_id)
        .with_context(|| format!("opening credential store for account '{account_id}'"))
}

/// Announces every touch of the OS credential store on stderr.
///
/// On macOS each access by an unsigned application raises a password dialog, so
/// the number and timing of these calls is the difference between one prompt and
/// an endless run of them. Reasoning about that from the outside turned out to
/// be guesswork; counting them is not.
///
/// Never logs a credential — only which account and which operation.
fn trace(operation: &str, account_id: &str) {
    eprintln!("[keychain] {operation} account={account_id}");
}

pub fn store_token(account_id: &str, token: &str) -> Result<()> {
    trace("write", account_id);
    entry(account_id)?
        .set_password(token)
        .with_context(|| format!("saving credential for account '{account_id}'"))
}

/// `None` when no credential exists, which is an ordinary first-run state rather
/// than a failure.
pub fn load_token(account_id: &str) -> Result<Option<String>> {
    trace("read", account_id);
    match entry(account_id)?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading credential for account '{account_id}'")),
    }
}

/// Removing a credential that was never stored is not an error — the desired end
/// state is "no credential", and it already holds.
pub fn delete_token(account_id: &str) -> Result<()> {
    trace("delete", account_id);
    match entry(account_id)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).with_context(|| format!("deleting credential for account '{account_id}'")),
    }
}
