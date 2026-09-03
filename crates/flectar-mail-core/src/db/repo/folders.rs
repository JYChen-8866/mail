use crate::error::Result;
use crate::models::FolderInfo;
use rusqlite::{Connection, OptionalExtension, Row, params};

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub imap_name: String,
    pub delimiter: Option<String>,
    pub role: Option<String>,
    pub uidvalidity: Option<i64>,
    pub uidnext: Option<i64>,
    pub highestmodseq: Option<i64>,
    pub last_seen_uid: i64,
    pub backfill_cursor: Option<i64>,
    pub backfill_done: bool,
    pub jmap_id: Option<String>,
}

fn from_row(row: &Row) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        imap_name: row.get("imap_name")?,
        delimiter: row.get("delimiter")?,
        role: row.get("role")?,
        uidvalidity: row.get("uidvalidity")?,
        uidnext: row.get("uidnext")?,
        highestmodseq: row.get("highestmodseq")?,
        last_seen_uid: row.get("last_seen_uid")?,
        backfill_cursor: row.get("backfill_cursor")?,
        backfill_done: row.get::<_, i64>("backfill_done")? != 0,
        jmap_id: row.get("jmap_id")?,
    })
}

pub fn upsert(
    conn: &Connection,
    account_id: i64,
    imap_name: &str,
    delimiter: Option<&str>,
    role: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO folders (account_id, imap_name, delimiter, role)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT(account_id, imap_name)
         DO UPDATE SET delimiter = excluded.delimiter, role = excluded.role, jmap_id = NULL",
        params![account_id, imap_name, delimiter, role],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND imap_name = ?2",
        params![account_id, imap_name],
        |r| r.get(0),
    )?;
    Ok(id)
}

pub fn list(conn: &Connection, account_id: Option<i64>) -> Result<Vec<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT f.id AS id, f.account_id AS account_id, f.imap_name AS imap_name,
                f.delimiter AS delimiter, f.role AS role, f.uidvalidity AS uidvalidity,
                f.uidnext AS uidnext, f.highestmodseq AS highestmodseq,
                f.last_seen_uid AS last_seen_uid, f.backfill_cursor AS backfill_cursor,
                f.backfill_done AS backfill_done, f.jmap_id AS jmap_id
         FROM folders f
         JOIN accounts a ON a.id = f.account_id
         WHERE (?1 IS NULL OR f.account_id = ?1)
           AND ((a.mail_protocol = 'jmap' AND f.jmap_id IS NOT NULL)
             OR (a.mail_protocol <> 'jmap' AND f.jmap_id IS NULL))
         ORDER BY f.id",
    )?;
    Ok(stmt
        .query_map(params![account_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn list_info(conn: &Connection, account_id: Option<i64>) -> Result<Vec<FolderInfo>> {
    Ok(list(conn, account_id)?
        .into_iter()
        .map(|f| FolderInfo {
            id: f.id,
            account_id: f.account_id,
            imap_name: f.imap_name,
            delimiter: f.delimiter,
            role: f.role,
        })
        .collect())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, imap_name, delimiter, role, uidvalidity, uidnext,
                highestmodseq, last_seen_uid, backfill_cursor, backfill_done, jmap_id
         FROM folders WHERE id = ?1",
    )?;
    Ok(stmt.query_row(params![id], from_row).optional()?)
}

pub fn by_role(conn: &Connection, account_id: i64, role: &str) -> Result<Option<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, imap_name, delimiter, role, uidvalidity, uidnext,
                highestmodseq, last_seen_uid, backfill_cursor, backfill_done, jmap_id
         FROM folders WHERE account_id = ?1 AND role = ?2 LIMIT 1",
    )?;
    Ok(stmt
        .query_row(params![account_id, role], from_row)
        .optional()?)
}

pub fn by_jmap_role(conn: &Connection, account_id: i64, role: &str) -> Result<Option<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, imap_name, delimiter, role, uidvalidity, uidnext,
                highestmodseq, last_seen_uid, backfill_cursor, backfill_done, jmap_id
         FROM folders
         WHERE account_id=?1 AND role=?2 AND jmap_id IS NOT NULL LIMIT 1",
    )?;
    Ok(stmt
        .query_row(params![account_id, role], from_row)
        .optional()?)
}

pub fn by_jmap_id(conn: &Connection, account_id: i64, jmap_id: &str) -> Result<Option<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, imap_name, delimiter, role, uidvalidity, uidnext,
                highestmodseq, last_seen_uid, backfill_cursor, backfill_done, jmap_id
         FROM folders WHERE account_id = ?1 AND jmap_id = ?2 LIMIT 1",
    )?;
    Ok(stmt
        .query_row(params![account_id, jmap_id], from_row)
        .optional()?)
}

pub fn upsert_jmap(
    conn: &Connection,
    account_id: i64,
    jmap_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<i64> {
    if let Some(folder) = by_jmap_id(conn, account_id, jmap_id)? {
        if let Some(conflict_id) = conn
            .query_row(
                "SELECT id FROM folders
                 WHERE account_id=?1 AND imap_name=?2 AND id<>?3 AND jmap_id IS NULL",
                params![account_id, name, folder.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            conn.execute(
                "UPDATE folders SET imap_name = imap_name || ' [removed ' || id || ']'
                 WHERE id=?1",
                params![conflict_id],
            )?;
        }
        let name = unique_jmap_name(conn, account_id, name, jmap_id, Some(folder.id))?;
        conn.execute(
            "UPDATE folders SET imap_name = ?2, role = ?3 WHERE id = ?1",
            params![folder.id, name, role],
        )?;
        return Ok(folder.id);
    }
    let name = unique_jmap_name(conn, account_id, name, jmap_id, None)?;
    conn.execute(
        "INSERT INTO folders (account_id, imap_name, delimiter, role, jmap_id,
                              backfill_done)
         VALUES (?1, ?2, '/', ?3, ?4, 1)
         ON CONFLICT(account_id, imap_name) DO UPDATE SET
           role = excluded.role, jmap_id = excluded.jmap_id, backfill_done = 1",
        params![account_id, name, role, jmap_id],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND jmap_id = ?2",
        params![account_id, jmap_id],
        |row| row.get(0),
    )?)
}

fn unique_jmap_name(
    conn: &Connection,
    account_id: i64,
    proposed: &str,
    jmap_id: &str,
    exclude_id: Option<i64>,
) -> Result<String> {
    let conflict: Option<Option<String>> = conn
        .query_row(
            "SELECT jmap_id FROM folders
             WHERE account_id=?1 AND imap_name=?2 AND (?3 IS NULL OR id<>?3)",
            params![account_id, proposed, exclude_id],
            |row| row.get(0),
        )
        .optional()?;
    if conflict.is_none() || conflict == Some(None) {
        return Ok(proposed.to_owned());
    }
    let short_id = jmap_id.chars().take(8).collect::<String>();
    let base = format!("{proposed} [{short_id}]");
    let mut candidate = base.clone();
    let mut suffix = 2;
    while conn
        .query_row(
            "SELECT 1 FROM folders
             WHERE account_id=?1 AND imap_name=?2 AND (?3 IS NULL OR id<>?3)",
            params![account_id, candidate, exclude_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        candidate = format!("{base} {suffix}");
        suffix += 1;
    }
    Ok(candidate)
}

pub fn set_uid_state(
    conn: &Connection,
    id: i64,
    uidvalidity: Option<i64>,
    uidnext: Option<i64>,
    highestmodseq: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE folders SET uidvalidity = ?2, uidnext = ?3, highestmodseq = ?4 WHERE id = ?1",
        params![id, uidvalidity, uidnext, highestmodseq],
    )?;
    Ok(())
}

pub fn set_last_seen_uid(conn: &Connection, id: i64, uid: i64) -> Result<()> {
    conn.execute(
        "UPDATE folders SET last_seen_uid = MAX(last_seen_uid, ?2) WHERE id = ?1",
        params![id, uid],
    )?;
    Ok(())
}

pub fn set_backfill(conn: &Connection, id: i64, cursor: Option<i64>, done: bool) -> Result<()> {
    conn.execute(
        "UPDATE folders SET backfill_cursor = ?2, backfill_done = ?3 WHERE id = ?1",
        params![id, cursor, done as i64],
    )?;
    Ok(())
}

/// Re-open historical work after an account's download window changes. Keep
/// the cursor so widening to All can continue below the oldest known UID.
pub fn reopen_backfill_for_account(conn: &Connection, account_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE folders SET backfill_done = 0 WHERE account_id = ?1",
        params![account_id],
    )?;
    Ok(())
}

/// UIDVALIDITY changed: drop all UID mappings for the folder (messages stay,
/// re-linked on next sync by Message-ID).
pub fn reset_uid_mappings(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE messages SET uid = NULL WHERE folder_id = ?1",
        params![id],
    )?;
    conn.execute(
        "UPDATE folders SET last_seen_uid = 0, uidnext = NULL, highestmodseq = NULL,
                            backfill_cursor = NULL, backfill_done = 0
         WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil;

    #[test]
    fn folder_listing_exposes_only_the_active_transport_projection() {
        let conn = testutil::conn();
        testutil::seed_account(&conn);
        conn.execute(
            "INSERT INTO folders(id,account_id,imap_name,role,jmap_id,backfill_done)
             VALUES(2,1,'JMAP Inbox','inbox','remote-inbox',1)",
            [],
        )
        .unwrap();

        assert_eq!(
            list(&conn, Some(1))
                .unwrap()
                .into_iter()
                .map(|folder| folder.id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        conn.execute("UPDATE accounts SET mail_protocol='jmap' WHERE id=1", [])
            .unwrap();
        assert_eq!(
            list(&conn, Some(1))
                .unwrap()
                .into_iter()
                .map(|folder| folder.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn imap_upsert_reclaims_a_same_name_folder_from_jmap() {
        let conn = testutil::conn();
        testutil::seed_account(&conn);
        conn.execute("UPDATE folders SET jmap_id='remote-inbox' WHERE id=1", [])
            .unwrap();
        assert_eq!(
            upsert(&conn, 1, "INBOX", Some("/"), Some("inbox")).unwrap(),
            1
        );
        assert!(get(&conn, 1).unwrap().unwrap().jmap_id.is_none());
    }

    #[test]
    fn distinct_jmap_mailboxes_with_the_same_display_path_are_not_merged() {
        let conn = testutil::conn();
        testutil::seed_account(&conn);
        let first = upsert_jmap(&conn, 1, "remote-one", "Projects / Release", None).unwrap();
        let second = upsert_jmap(&conn, 1, "remote-two", "Projects / Release", None).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            by_jmap_id(&conn, 1, "remote-one").unwrap().unwrap().id,
            first
        );
        assert_eq!(
            by_jmap_id(&conn, 1, "remote-two").unwrap().unwrap().id,
            second
        );
        assert_ne!(
            get(&conn, first).unwrap().unwrap().imap_name,
            get(&conn, second).unwrap().unwrap().imap_name
        );
    }
}
