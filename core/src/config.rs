//! Account configuration.
//!
//! Credentials are read from a file the user writes themselves, or from the
//! environment — they are never entered through the app and never travel
//! through anything but this process. For a personal build a plaintext file
//! under %APPDATA% is an honest trade; moving it to the Windows Credential
//! Manager (DPAPI) is a task in its own right and is deliberately not faked here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub id: String,
    pub label: String,
    pub color: String,
    pub identity: String,
    /// JMAP session endpoint. Fastmail: https://api.fastmail.com/jmap/session
    #[serde(default = "default_session_url")]
    pub session_url: String,
    /// The API token itself. Prefer `token_env` if you would rather it not sit
    /// on disk.
    #[serde(default)]
    pub token: Option<String>,
    /// Name of an environment variable holding the token. Takes precedence.
    #[serde(default)]
    pub token_env: Option<String>,
    /// Present when the account was connected with OAuth. Not a secret — a
    /// public client has no secret to keep — so it lives here rather than in the
    /// credential store. Its presence is what marks the account as OAuth: the
    /// stored credential is then a refresh token rather than an API token.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Present when the account speaks IMAP rather than JMAP. Its presence is
    /// what selects the backend; the stored credential is then the app-specific
    /// password rather than a token.
    #[serde(default)]
    pub imap: Option<ImapConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
}

impl AccountConfig {
    pub fn is_oauth(&self) -> bool {
        self.client_id.is_some()
    }

    pub fn is_imap(&self) -> bool {
        self.imap.is_some()
    }
}

pub fn default_session_url() -> String {
    "https://api.fastmail.com/jmap/session".to_string()
}

impl AccountConfig {
    /// Environment first (useful for CI and for keeping a token out of any store
    /// at all), then the OS credential store, then the legacy inline field.
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(var) = &self.token_env {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    return Ok(value);
                }
            }
        }
        if let Some(token) = crate::secrets::load_token(&self.id)? {
            if !token.trim().is_empty() {
                return Ok(token);
            }
        }
        if let Some(token) = self.token.as_ref().filter(|t| !t.trim().is_empty()) {
            return Ok(token.clone());
        }
        anyhow::bail!(
            "no credential stored for account '{}' — sign in again to reconnect it",
            self.id
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
}

impl Config {
    /// %APPDATA%\BazMail on Windows, ~/.config/bazmail elsewhere.
    pub fn config_dir() -> Result<PathBuf> {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Ok(PathBuf::from(appdata).join("BazMail"));
        }
        let home = std::env::var("HOME").context("neither APPDATA nor HOME is set")?;
        Ok(PathBuf::from(home).join(".config").join("bazmail"))
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    /// Missing config is not an error — it is the first-run state.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes a commented example the user can fill in. Never overwrites.

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path()?)
    }

    /// Written via a temporary file and renamed into place.
    ///
    /// A direct write leaves a truncated config if anything interrupts it, and
    /// this file is the only record of which accounts exist — losing it means
    /// the app forgets every account while their credentials sit orphaned in the
    /// OS store, which is exactly the state this bug produced.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(self)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, &body).with_context(|| format!("writing {}", temp.display()))?;
        std::fs::rename(&temp, path)
            .with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Adds or replaces an account by id. Replacing is how re-authenticating
    /// works, so it must not duplicate the entry.
    pub fn upsert(&mut self, account: AccountConfig) {
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => self.accounts.push(account),
        }
    }

    pub fn remove(&mut self, account_id: &str) {
        self.accounts.retain(|a| a.id != account_id);
    }
}

/// Derives a stable, filesystem- and keyring-safe id from an address, so
/// `you@example.com` becomes `example`. Falls back to the local part when
/// there is no domain to read.
pub fn account_id_from_address(address: &str) -> String {
    let domain = address.split('@').nth(1).unwrap_or(address);
    let stem = domain.split('.').next().unwrap_or(domain);
    let cleaned: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if cleaned.is_empty() {
        "account".to_string()
    } else {
        cleaned.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> AccountConfig {
        AccountConfig {
            id: id.into(),
            label: format!("{id}.com"),
            color: "#5b7cfa".into(),
            identity: format!("me@{id}.com"),
            session_url: default_session_url(),
            token: None,
            token_env: None,
            client_id: None,
            imap: None,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bazmail-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn oauth_details_survive_a_round_trip() {
        let path = temp_path("oauth");
        let mut config = Config::default();
        let mut a = account("example");
        a.client_id = Some("68ae41ae".into());
        config.upsert(a);
        config.save_to(&path).unwrap();

        // The bug this pins: a client_id that fails to persist makes the account
        // look like an API-token account on the next start, and the refresh token
        // then gets sent as a bearer token.
        let reloaded = Config::load_from(&path).unwrap();
        assert_eq!(reloaded.accounts.len(), 1);
        assert_eq!(reloaded.accounts[0].client_id.as_deref(), Some("68ae41ae"));
        assert!(reloaded.accounts[0].is_oauth());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn imap_details_survive_a_round_trip() {
        let path = temp_path("imap");
        let mut config = Config::default();
        let mut a = account("icloud");
        a.imap = Some(ImapConfig {
            host: "imap.mail.me.com".into(),
            port: 993,
            username: "me@example.com".into(),
        });
        config.upsert(a);
        config.save_to(&path).unwrap();

        let reloaded = Config::load_from(&path).unwrap();
        let imap = reloaded.accounts[0].imap.as_ref().unwrap();
        assert_eq!(imap.host, "imap.mail.me.com");
        assert_eq!(imap.port, 993);
        assert!(reloaded.accounts[0].is_imap());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_rather_than_duplicating() {
        let mut config = Config::default();
        config.upsert(account("example"));
        let mut updated = account("example");
        updated.client_id = Some("new".into());
        config.upsert(updated);

        // Re-authenticating must not leave two entries fighting over one id.
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.accounts[0].client_id.as_deref(), Some("new"));
    }

    #[test]
    fn two_accounts_both_persist() {
        let path = temp_path("two");
        let mut config = Config::default();
        config.upsert(account("example"));
        config.upsert(account("icloud"));
        config.save_to(&path).unwrap();

        assert_eq!(Config::load_from(&path).unwrap().accounts.len(), 2);
        let _ = std::fs::remove_file(&path);
    }
}
