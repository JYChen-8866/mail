use crate::error::{CoreError, Result};
use rusqlite::Connection;

// The application is pre-release, so the development schema stays canonical
// instead of carrying forward migrations for profiles that can be recreated.
const MIGRATIONS: &[&str] = &[include_str!("migrations/001_init.sql")];
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn run(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let latest = LATEST_VERSION;
    if version > latest {
        return Err(CoreError::Other(format!(
            "unsupported pre-release mail database version {version}; recreate the development profile"
        )));
    }

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let target = (index + 1) as i64;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
        tracing::info!("applied mail db migration {target}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::collections::BTreeSet;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run(&mut conn).unwrap();
        conn
    }

    fn application_tables(conn: &Connection) -> BTreeSet<String> {
        conn.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table'
               AND name NOT LIKE 'sqlite_%'
               AND name NOT LIKE 'messages_fts_%'
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn seed_mail_graph(conn: &Connection) {
        conn.execute(
            "INSERT INTO accounts (
               id, email, provider, auth_kind, username, imap_host, imap_port,
               smtp_host, smtp_port, created_at
             ) VALUES (1, 'me@test.dev', 'imap', 'password', 'me', 'h', 993, 'h', 587, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (id, account_id, imap_name, role)
             VALUES (1, 1, 'INBOX', 'inbox'), (2, 1, 'STARRED', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm) VALUES (1, 1, 'schema')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (
               id, account_id, thread_id, folder_id, uid, subject, from_addr,
               to_json, cc_json, date, snippet
             ) VALUES (
               1, 1, 1, 1, 7, 'Schema contract', 'sender@test.dev', '[]', '[]', 1,
               'freshprofileterm'
             )",
            [],
        )
        .unwrap();
    }

    #[test]
    fn fresh_schema_has_the_canonical_mail_contract() {
        let conn = fresh();
        let expected = [
            "accounts",
            "ai_usage_events",
            "app_settings",
            "attachments",
            "contact_accounts",
            "contacts",
            "cross_store_operations",
            "draft_attachments",
            "drafts_meta",
            "folders",
            "gmail_labels",
            "gmail_sync_state",
            "jmap_sync_state",
            "labels",
            "message_bodies",
            "message_embeddings",
            "message_folders",
            "message_labels",
            "message_refs",
            "messages",
            "messages_fts",
            "notification_outbox",
            "pending_actions",
            "route_cache",
            "snippets",
            "snoozes",
            "split_rules",
            "sync_failures",
            "threads",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(application_tables(&conn), expected);
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );

        for forbidden in ["calendar_events", "calendars", "caldav_config"] {
            assert!(!application_tables(&conn).contains(forbidden));
        }
        let seeded: i64 = conn
            .query_row("SELECT COUNT(*) FROM labels WHERE is_auto = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(seeded, 4);

        let violations = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_some();
        assert!(!violations);
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }

    #[test]
    fn fresh_schema_preserves_fts_contact_and_gmail_constraints() {
        let conn = fresh();
        seed_mail_graph(&conn);
        conn.execute(
            "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
             VALUES (1, 'Schema contract', 'sender@test.dev', '', 'freshprofileterm')",
            [],
        )
        .unwrap();
        let matches = |term: &str| {
            conn.query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?1",
                params![term],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(matches("freshprofileterm"), 1);
        conn.execute("DELETE FROM messages_fts WHERE rowid = 1", [])
            .unwrap();
        assert_eq!(matches("freshprofileterm"), 0);

        conn.execute(
            "INSERT INTO contacts (email) VALUES ('Person@Example.com')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO contacts (email) VALUES ('person@example.com')",
                []
            )
            .is_err()
        );

        conn.execute(
            "INSERT INTO message_folders (message_id, folder_id) VALUES (1, 1), (1, 2)",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO message_folders (message_id, folder_id) VALUES (1, 2)",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn pre_release_profiles_are_rejected_without_mutation() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 28).unwrap();
        let error = run(&mut conn).unwrap_err().to_string();
        assert!(error.contains("recreate the development profile"));
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            28
        );
        assert!(application_tables(&conn).is_empty());
    }
}
