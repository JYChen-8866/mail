//! Coherent online snapshots of the physically separate mail and calendar
//! stores. Both writer queues are held at the same coordinated gate while
//! SQLite's online-backup API copies committed pages, so no application write
//! can land between the two captured database states.

use super::Db;
use crate::error::{CoreError, Result};
use rusqlite::{Connection, MAIN_DB, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

const SNAPSHOT_FORMAT: &str = "flectar-mail-database-snapshot";
const SNAPSHOT_VERSION: u32 = 1;
const MAIL_FILE: &str = "mail.sqlite3";
const CALENDAR_FILE: &str = "calendar.sqlite3";
const MANIFEST_FILE: &str = "manifest.json";
const GATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const BACKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 60);

fn coordination_error(phase: &str, error: impl std::fmt::Display) -> CoreError {
    CoreError::Other(format!(
        "database snapshot {phase} coordination failed: {error}"
    ))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSnapshotStore {
    pub file: String,
    pub schema_version: i64,
    pub sqlite_version: String,
    pub page_count: i64,
    pub page_size: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSnapshotManifest {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    pub core_version: String,
    pub mail: DatabaseSnapshotStore,
    pub calendar: DatabaseSnapshotStore,
}

fn snapshot_store(
    conn: &Connection,
    destination: &Path,
    file: &str,
) -> Result<DatabaseSnapshotStore> {
    let schema_version = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let sqlite_version = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let page_count = conn.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let page_size = conn.pragma_query_value(None, "page_size", |row| row.get(0))?;
    conn.backup(MAIN_DB, destination, None)?;
    Ok(DatabaseSnapshotStore {
        file: file.to_owned(),
        schema_version,
        sqlite_version,
        page_count,
        page_size,
    })
}

fn verify_store(path: &Path, expected: &DatabaseSnapshotStore, store: &str) -> Result<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let schema: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema != expected.schema_version {
        return Err(CoreError::Other(format!(
            "{store} snapshot schema changed during verification: expected {}, found {schema}",
            expected.schema_version
        )));
    }
    let page_count: i64 = conn.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let page_size: i64 = conn.pragma_query_value(None, "page_size", |row| row.get(0))?;
    if page_count != expected.page_count || page_size != expected.page_size {
        return Err(CoreError::Other(format!(
            "{store} snapshot page geometry changed during verification: expected {} pages of {} bytes, found {page_count} pages of {page_size} bytes",
            expected.page_count, expected.page_size
        )));
    }
    let integrity: String = conn.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if !integrity.eq_ignore_ascii_case("ok") {
        return Err(CoreError::Other(format!(
            "{store} snapshot integrity_check failed: {integrity}"
        )));
    }
    let foreign_key_violation: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
        [],
        |row| row.get(0),
    )?;
    if foreign_key_violation {
        return Err(CoreError::Other(format!(
            "{store} snapshot contains a foreign-key violation"
        )));
    }
    Ok(())
}

fn staging_path(destination: &Path) -> Result<PathBuf> {
    let parent = destination.parent().ok_or_else(|| {
        CoreError::Other("database snapshot destination has no parent directory".into())
    })?;
    let name = destination.file_name().ok_or_else(|| {
        CoreError::Other("database snapshot destination has no directory name".into())
    })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CoreError::Other(format!("system clock before Unix epoch: {error}")))?
        .as_nanos();
    Ok(parent.join(format!(
        ".{}.partial-{}-{nonce}",
        name.to_string_lossy(),
        std::process::id()
    )))
}

#[cfg(unix)]
fn restrict_permissions(directory: &Path, files: &[&Path]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    for file in files {
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_directory: &Path, _files: &[&Path]) -> Result<()> {
    Ok(())
}

/// Create a new snapshot directory atomically. The destination must not
/// already exist, which prevents an export from overwriting an older backup.
pub async fn create(
    mail_db: &Db,
    calendar_db: &Db,
    destination: &Path,
) -> Result<DatabaseSnapshotManifest> {
    if destination.exists() {
        return Err(CoreError::Other(format!(
            "snapshot destination already exists: {}",
            destination.display()
        )));
    }
    let parent = destination.parent().ok_or_else(|| {
        CoreError::Other("database snapshot destination has no parent directory".into())
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let staging = staging_path(destination)?;
    tokio::fs::create_dir(&staging).await?;
    // Restrict access before SQLite creates any files. Applying permissions
    // only after the backup would briefly expose private mail under a common
    // 022 umask.
    restrict_permissions(&staging, &[])?;

    let mail_path = staging.join(MAIL_FILE);
    let calendar_path = staging.join(CALENDAR_FILE);
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let mail_ready = ready_tx.clone();
    let calendar_ready = ready_tx;
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let mail_done = done_tx.clone();
    let calendar_done = done_tx;
    let (mail_start_tx, mail_start_rx) = mpsc::sync_channel::<()>(0);
    let (calendar_start_tx, calendar_start_rx) = mpsc::sync_channel::<()>(0);
    let (mail_release_tx, mail_release_rx) = mpsc::sync_channel::<()>(0);
    let (calendar_release_tx, calendar_release_rx) = mpsc::sync_channel::<()>(0);
    let mail_destination = mail_path.clone();
    let calendar_destination = calendar_path.clone();

    let coordinator = tokio::task::spawn_blocking(move || -> Result<()> {
        for _ in 0..2 {
            ready_rx
                .recv_timeout(GATE_TIMEOUT)
                .map_err(|error| coordination_error("ready", error))?;
        }
        mail_start_tx
            .send(())
            .map_err(|error| coordination_error("mail start", error))?;
        calendar_start_tx
            .send(())
            .map_err(|error| coordination_error("calendar start", error))?;
        for _ in 0..2 {
            done_rx
                .recv_timeout(BACKUP_TIMEOUT)
                .map_err(|error| coordination_error("completion", error))?;
        }
        mail_release_tx
            .send(())
            .map_err(|error| coordination_error("mail release", error))?;
        calendar_release_tx
            .send(())
            .map_err(|error| coordination_error("calendar release", error))?;
        Ok(())
    });

    let (mail, calendar, coordination) = tokio::join!(
        mail_db.write(move |conn| {
            mail_ready
                .send(())
                .map_err(|error| coordination_error("mail ready", error))?;
            mail_start_rx
                .recv_timeout(GATE_TIMEOUT)
                .map_err(|error| coordination_error("mail start", error))?;
            let result = snapshot_store(conn, &mail_destination, MAIL_FILE);
            mail_done
                .send(())
                .map_err(|error| coordination_error("mail completion", error))?;
            mail_release_rx
                .recv_timeout(BACKUP_TIMEOUT)
                .map_err(|error| coordination_error("mail release", error))?;
            result
        }),
        calendar_db.write(move |conn| {
            calendar_ready
                .send(())
                .map_err(|error| coordination_error("calendar ready", error))?;
            calendar_start_rx
                .recv_timeout(GATE_TIMEOUT)
                .map_err(|error| coordination_error("calendar start", error))?;
            let result = snapshot_store(conn, &calendar_destination, CALENDAR_FILE);
            calendar_done
                .send(())
                .map_err(|error| coordination_error("calendar completion", error))?;
            calendar_release_rx
                .recv_timeout(BACKUP_TIMEOUT)
                .map_err(|error| coordination_error("calendar release", error))?;
            result
        }),
        coordinator
    );

    let result = async {
        coordination.map_err(|error| coordination_error("task", error))??;
        let mail = mail?;
        let calendar = calendar?;
        verify_store(&mail_path, &mail, "mail")?;
        verify_store(&calendar_path, &calendar, "calendar")?;
        let manifest = DatabaseSnapshotManifest {
            format: SNAPSHOT_FORMAT.to_owned(),
            version: SNAPSHOT_VERSION,
            created_at: chrono::Utc::now().to_rfc3339(),
            core_version: env!("CARGO_PKG_VERSION").to_owned(),
            mail,
            calendar,
        };
        let manifest_path = staging.join(MANIFEST_FILE);
        tokio::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?).await?;
        restrict_permissions(&staging, &[&mail_path, &calendar_path, &manifest_path])?;
        tokio::fs::rename(&staging, destination).await?;
        Ok(manifest)
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{calendar_migrations, migrations};

    #[tokio::test]
    async fn snapshot_is_complete_verified_and_never_overwrites() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let mail = Db::open(&source.path().join("mail.db")).unwrap();
        let calendar = Db::open_calendar(&source.path().join("calendar.db")).unwrap();
        mail.write(|conn| {
            conn.execute(
                "INSERT INTO accounts (
                   id, email, provider, auth_kind, username, imap_host, imap_port,
                   smtp_host, smtp_port, created_at
                 ) VALUES (1, 'snapshot@example.test', 'imap', 'password', 'snapshot',
                           'imap.example.test', 993, 'smtp.example.test', 465, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        calendar
            .write(|conn| {
                conn.execute(
                    "INSERT INTO calendars (id, account_id, url, display_name)
                     VALUES (1, 1, 'local://snapshot', 'Snapshot')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let destination = output.path().join("snapshot");
        let manifest = create(&mail, &calendar, &destination).await.unwrap();
        assert_eq!(manifest.mail.schema_version, migrations::LATEST_VERSION);
        assert_eq!(
            manifest.calendar.schema_version,
            calendar_migrations::LATEST_VERSION
        );

        let stored: DatabaseSnapshotManifest =
            serde_json::from_slice(&std::fs::read(destination.join(MANIFEST_FILE)).unwrap())
                .unwrap();
        assert_eq!(stored, manifest);
        let copied_mail = Connection::open(destination.join(MAIL_FILE)).unwrap();
        let copied_calendar = Connection::open(destination.join(CALENDAR_FILE)).unwrap();
        assert_eq!(
            copied_mail
                .query_row("SELECT COUNT(*) FROM accounts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            copied_calendar
                .query_row("SELECT COUNT(*) FROM calendars", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );

        let error = create(&mail, &calendar, &destination)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"));
    }
}
