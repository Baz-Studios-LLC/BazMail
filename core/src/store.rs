//! The local mirror.
//!
//! Every read the UI performs comes from here, never from the network — that is
//! what makes navigation instant and what makes the app work on a plane. Sync
//! writes into this; the UI never waits on sync.
//!
//! Message identity is deliberately the backend id *for now*. The internal-UUID
//! mapping described in the architecture notes lands with the second backend,
//! since it only earns its keep once ids can collide across accounts.

use crate::model::{Envelope, Mailbox, Mutation};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;

            // Mark the directory first, so SQLite's -wal and -shm sidecars are
            // covered as they appear rather than being left in the clear.
            if let Err(e) = crate::at_rest::protect(parent) {
                eprintln!("WARNING: store is NOT encrypted at rest: {e}");
            }
        }

        // An existing store predates this and is still plaintext; encrypting the
        // directory does not reach back and cover it.
        if path.exists() && crate::at_rest::is_plaintext_sqlite(path) {
            if let Err(e) = crate::at_rest::protect(path) {
                eprintln!("WARNING: existing store is NOT encrypted at rest: {e}");
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening store at {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory()?,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS mailboxes (
                account_id     TEXT NOT NULL,
                id             TEXT NOT NULL,
                name           TEXT NOT NULL,
                role           TEXT,
                total_emails   INTEGER NOT NULL DEFAULT 0,
                unread_emails  INTEGER NOT NULL DEFAULT 0,
                sort_order     INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, id)
            );

            CREATE TABLE IF NOT EXISTS envelopes (
                account_id     TEXT NOT NULL,
                id             TEXT NOT NULL,
                thread_id      TEXT NOT NULL,
                mailbox_ids    TEXT NOT NULL,
                from_json      TEXT NOT NULL,
                to_json        TEXT NOT NULL,
                subject        TEXT NOT NULL,
                preview        TEXT NOT NULL,
                received_at    TEXT NOT NULL,
                is_unread      INTEGER NOT NULL,
                is_flagged     INTEGER NOT NULL,
                has_attachment INTEGER NOT NULL,
                PRIMARY KEY (account_id, id)
            );

            CREATE INDEX IF NOT EXISTS envelopes_by_time
                ON envelopes (account_id, received_at DESC);

            -- Changes made locally that have not reached the server yet.
            -- Rows live only until the send succeeds; a row that keeps failing
            -- stays put with its error so it is visible rather than lost.
            CREATE TABLE IF NOT EXISTS outbox (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id   TEXT NOT NULL,
                email_id     TEXT NOT NULL,
                kind         TEXT NOT NULL,
                add_ids      TEXT NOT NULL,
                remove_ids   TEXT NOT NULL,
                created_at   TEXT NOT NULL,
                attempts     INTEGER NOT NULL DEFAULT 0,
                last_error   TEXT
            );
            "#,
        )?;

        // Added after the table shipped, so CREATE TABLE IF NOT EXISTS will not
        // introduce them on an existing store. A duplicate-column error here
        // means the migration already ran, which is success, not failure.
        for column in [
            "ALTER TABLE outbox ADD COLUMN add_flags TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE outbox ADD COLUMN remove_flags TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE envelopes ADD COLUMN verified_domain TEXT",
            "ALTER TABLE envelopes ADD COLUMN message_id TEXT",
            "ALTER TABLE envelopes ADD COLUMN references_json TEXT NOT NULL DEFAULT '[]'",
        ] {
            let _ = self.conn.execute(column, []);
        }
        Ok(())
    }

    /// Marks a message read or unread in the mirror.
    pub fn set_unread(&self, account_id: &str, email_id: &str, is_unread: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE envelopes SET is_unread = ?1 WHERE account_id = ?2 AND id = ?3",
            params![is_unread as i32, account_id, email_id],
        )?;
        Ok(())
    }

    pub fn put_mailboxes(&self, mailboxes: &[Mailbox]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO mailboxes
                (account_id, id, name, role, total_emails, unread_emails, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account_id, id) DO UPDATE SET
                name = excluded.name,
                role = excluded.role,
                total_emails = excluded.total_emails,
                unread_emails = excluded.unread_emails,
                sort_order = excluded.sort_order",
        )?;
        for m in mailboxes {
            stmt.execute(params![
                m.account_id,
                m.id,
                m.name,
                m.role,
                m.total_emails,
                m.unread_emails,
                m.sort_order
            ])?;
        }
        Ok(())
    }

    pub fn mailboxes(&self, account_id: &str) -> Result<Vec<Mailbox>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT account_id, id, name, role, total_emails, unread_emails, sort_order
             FROM mailboxes WHERE account_id = ?1 ORDER BY sort_order, name",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(Mailbox {
                account_id: row.get(0)?,
                id: row.get(1)?,
                name: row.get(2)?,
                role: row.get(3)?,
                total_emails: row.get(4)?,
                unread_emails: row.get(5)?,
                sort_order: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn put_envelopes(&self, envelopes: &[Envelope]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO envelopes
                (account_id, id, thread_id, mailbox_ids, from_json, to_json,
                 subject, preview, received_at, is_unread, is_flagged, has_attachment,
                 verified_domain, message_id, references_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(account_id, id) DO UPDATE SET
                thread_id = excluded.thread_id,
                mailbox_ids = excluded.mailbox_ids,
                from_json = excluded.from_json,
                to_json = excluded.to_json,
                subject = excluded.subject,
                preview = excluded.preview,
                received_at = excluded.received_at,
                is_unread = excluded.is_unread,
                is_flagged = excluded.is_flagged,
                has_attachment = excluded.has_attachment,
                verified_domain = excluded.verified_domain,
                message_id = excluded.message_id,
                references_json = excluded.references_json",
        )?;
        for e in envelopes {
            stmt.execute(params![
                e.account_id,
                e.id,
                e.thread_id,
                serde_json::to_string(&e.mailbox_ids)?,
                serde_json::to_string(&e.from)?,
                serde_json::to_string(&e.to)?,
                e.subject,
                e.preview,
                e.received_at,
                e.is_unread as i32,
                e.is_flagged as i32,
                e.has_attachment as i32,
                e.verified_domain,
                e.message_id,
                serde_json::to_string(&e.references).unwrap_or_else(|_| "[]".into()),
            ])?;
        }
        Ok(())
    }

    /// Envelopes across every account, newest first — the unified inbox read.
    /// `mailbox_ids` narrows to specific mailboxes; empty means all of them.
    pub fn envelopes(&self, mailbox_ids: &[String], limit: usize) -> Result<Vec<Envelope>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT account_id, id, thread_id, mailbox_ids, from_json, to_json,
                    subject, preview, received_at, is_unread, is_flagged, has_attachment,
                    verified_domain, message_id, references_json
             FROM envelopes ORDER BY received_at DESC LIMIT ?1",
        )?;
        // Over-fetch, then filter in Rust: mailbox membership is a JSON array, and
        // a proper join table is the right fix once this stops being fast enough.
        let over_fetch = if mailbox_ids.is_empty() {
            limit
        } else {
            limit * 8
        };
        let rows = stmt.query_map(params![over_fetch as i64], |row| {
            let mailboxes: String = row.get(3)?;
            let from: String = row.get(4)?;
            let to: String = row.get(5)?;
            Ok(Envelope {
                account_id: row.get(0)?,
                id: row.get(1)?,
                thread_id: row.get(2)?,
                mailbox_ids: serde_json::from_str(&mailboxes).unwrap_or_default(),
                from: serde_json::from_str(&from).unwrap_or_default(),
                to: serde_json::from_str(&to).unwrap_or_default(),
                subject: row.get(6)?,
                preview: row.get(7)?,
                received_at: row.get(8)?,
                is_unread: row.get::<_, i32>(9)? != 0,
                is_flagged: row.get::<_, i32>(10)? != 0,
                has_attachment: row.get::<_, i32>(11)? != 0,
                verified_domain: row.get(12)?,
                message_id: row.get(13)?,
                references: row
                    .get::<_, String>(14)
                    .ok()
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
            })
        })?;

        let mut out = Vec::new();
        for envelope in rows {
            let envelope = envelope?;
            let keep = mailbox_ids.is_empty()
                || envelope.mailbox_ids.iter().any(|id| mailbox_ids.contains(id));
            if keep {
                out.push(envelope);
            }
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Applies a mailbox change to the mirror immediately.
    ///
    /// This is the optimistic half: the row moves before the server is told, so
    /// the list updates on the same frame as the keypress. Sync reconciles later
    /// and the server wins if they disagree.
    pub fn apply_mailbox_change(
        &self,
        account_id: &str,
        email_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let current: String = self.conn.query_row(
            "SELECT mailbox_ids FROM envelopes WHERE account_id = ?1 AND id = ?2",
            params![account_id, email_id],
            |row| row.get(0),
        )?;

        let mut ids: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
        ids.retain(|id| !remove.contains(id));
        for id in add {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }

        self.conn.execute(
            "UPDATE envelopes SET mailbox_ids = ?1 WHERE account_id = ?2 AND id = ?3",
            params![serde_json::to_string(&ids)?, account_id, email_id],
        )?;
        Ok(())
    }

    pub fn enqueue(
        &self,
        account_id: &str,
        email_id: &str,
        kind: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<i64> {
        self.enqueue_full(account_id, email_id, kind, add, remove, &[], &[])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_full(
        &self,
        account_id: &str,
        email_id: &str,
        kind: &str,
        add: &[String],
        remove: &[String],
        add_flags: &[String],
        remove_flags: &[String],
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO outbox
                (account_id, email_id, kind, add_ids, remove_ids, add_flags, remove_flags, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                email_id,
                kind,
                serde_json::to_string(add)?,
                serde_json::to_string(remove)?,
                serde_json::to_string(add_flags)?,
                serde_json::to_string(remove_flags)?,
                now_rfc3339(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Oldest first: mutations to the same message must land in the order they
    /// were made, or an archive followed by an undo could arrive reversed.
    pub fn pending(&self, limit: usize) -> Result<Vec<Mutation>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, account_id, email_id, kind, add_ids, remove_ids, attempts,
                    add_flags, remove_flags
             FROM outbox ORDER BY id ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let add: String = row.get(4)?;
            let remove: String = row.get(5)?;
            let add_flags: String = row.get(7)?;
            let remove_flags: String = row.get(8)?;
            Ok(Mutation {
                id: row.get(0)?,
                account_id: row.get(1)?,
                email_id: row.get(2)?,
                kind: row.get(3)?,
                add_mailbox_ids: serde_json::from_str(&add).unwrap_or_default(),
                remove_mailbox_ids: serde_json::from_str(&remove).unwrap_or_default(),
                add_flags: serde_json::from_str(&add_flags).unwrap_or_default(),
                remove_flags: serde_json::from_str(&remove_flags).unwrap_or_default(),
                attempts: row.get::<_, i64>(6)? as u32,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn pending_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    pub fn drop_mutation(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn record_failure(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET attempts = attempts + 1, last_error = ?1 WHERE id = ?2",
            params![error, id],
        )?;
        Ok(())
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EmailAddress;

    fn envelope(id: &str, received_at: &str, mailbox: &str) -> Envelope {
        Envelope {
            id: id.into(),
            account_id: "acct".into(),
            thread_id: format!("t-{id}"),
            mailbox_ids: vec![mailbox.into()],
            from: vec![EmailAddress {
                name: Some("Sarah Chen".into()),
                email: "sarah@loopstudios.example".into(),
            }],
            to: vec![],
            subject: "Q3 invoice".into(),
            preview: "Attached the revised one".into(),
            received_at: received_at.into(),
            is_unread: true,
            is_flagged: false,
            has_attachment: true,
            verified_domain: None,
            message_id: None,
            references: Vec::new(),
        }
    }

    #[test]
    fn envelopes_round_trip_newest_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .put_envelopes(&[
                envelope("a", "2026-08-28T09:00:00Z", "inbox"),
                envelope("b", "2026-08-30T09:00:00Z", "inbox"),
            ])
            .unwrap();

        let found = store.envelopes(&[], 10).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "b", "newest should sort first");
        assert_eq!(found[0].from[0].display(), "Sarah Chen");
    }

    #[test]
    fn envelopes_filter_by_mailbox() {
        let store = Store::open_in_memory().unwrap();
        store
            .put_envelopes(&[
                envelope("a", "2026-08-28T09:00:00Z", "inbox"),
                envelope("b", "2026-08-30T09:00:00Z", "archive"),
            ])
            .unwrap();

        let found = store.envelopes(&["inbox".to_string()], 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a");
    }

    #[test]
    fn apply_mailbox_change_moves_the_message() {
        let store = Store::open_in_memory().unwrap();
        store
            .put_envelopes(&[envelope("a", "2026-08-30T09:00:00Z", "inbox")])
            .unwrap();

        store
            .apply_mailbox_change("acct", "a", &["archive".into()], &["inbox".into()])
            .unwrap();

        assert!(
            store.envelopes(&["inbox".to_string()], 10).unwrap().is_empty(),
            "archived mail must leave the inbox view immediately"
        );
        assert_eq!(store.envelopes(&["archive".to_string()], 10).unwrap().len(), 1);
    }

    #[test]
    fn apply_mailbox_change_leaves_other_mailboxes_alone() {
        let store = Store::open_in_memory().unwrap();
        let mut e = envelope("a", "2026-08-30T09:00:00Z", "inbox");
        e.mailbox_ids.push("starred".into());
        store.put_envelopes(&[e]).unwrap();

        store
            .apply_mailbox_change("acct", "a", &["archive".into()], &["inbox".into()])
            .unwrap();

        let found = store.envelopes(&["starred".to_string()], 10).unwrap();
        assert_eq!(found.len(), 1, "unrelated memberships must survive a move");
        assert!(found[0].mailbox_ids.contains(&"archive".to_string()));
    }

    #[test]
    fn outbox_drains_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .enqueue("acct", "a", "archive", &["archive".into()], &["inbox".into()])
            .unwrap();
        let second = store
            .enqueue("acct", "a", "unarchive", &["inbox".into()], &["archive".into()])
            .unwrap();

        let pending = store.pending(10).unwrap();
        assert_eq!(pending.len(), 2);
        // An archive and its undo arriving reversed would leave the server in the
        // opposite state to the mirror, so order is not incidental here.
        assert_eq!(pending[0].id, first);
        assert_eq!(pending[1].id, second);
        assert_eq!(pending[0].kind, "archive");
    }

    #[test]
    fn a_sent_mutation_leaves_the_queue_and_a_failed_one_stays() {
        let store = Store::open_in_memory().unwrap();
        let sent = store
            .enqueue("acct", "a", "archive", &["archive".into()], &["inbox".into()])
            .unwrap();
        let stuck = store
            .enqueue("acct", "b", "archive", &["archive".into()], &["inbox".into()])
            .unwrap();

        store.drop_mutation(sent).unwrap();
        store.record_failure(stuck, "network unreachable").unwrap();

        let pending = store.pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, stuck);
        assert_eq!(pending[0].attempts, 1, "a retry must be counted, not lost");
        assert_eq!(store.pending_count().unwrap(), 1);
    }

    #[test]
    fn put_envelopes_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let e = envelope("a", "2026-08-28T09:00:00Z", "inbox");
        store.put_envelopes(&[e.clone()]).unwrap();
        store.put_envelopes(&[e]).unwrap();
        assert_eq!(store.envelopes(&[], 10).unwrap().len(), 1);
    }
}
