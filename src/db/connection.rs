use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::migrations;

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (or create) the database at `path`, applying schema migrations as needed.
    pub fn open(path: &Path) -> Result<Self, anyhow::Error> {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    "Failed to create DB directory '{}': {}",
                    parent.display(),
                    e
                );
            }
        }

        // Clean orphaned WAL/SHM files (e.g. after manual DB deletion or crash)
        // SQLite auto-recovers in WAL mode, but stray files waste disk space.
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let shm = PathBuf::from(format!("{}-shm", path.display()));
        if !path.exists() {
            std::fs::remove_file(&wal).ok();
            std::fs::remove_file(&shm).ok();
        }

        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| anyhow::anyhow!("Failed to read DB user_version: {}", e))?;
        if version > migrations::CURRENT_USER_VERSION {
            anyhow::bail!(
                "Database schema version {} is newer than this CCSwitch build supports ({})",
                version,
                migrations::CURRENT_USER_VERSION
            );
        }
        if version < migrations::CURRENT_USER_VERSION {
            tracing::info!(
                "Applying DB migrations v{} → v{}",
                version,
                migrations::CURRENT_USER_VERSION
            );
            migrations::apply_migrations(&conn)?;
        }

        set_private_permissions(path);
        set_private_permissions(&wal);
        set_private_permissions(&shm);
        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    if path.exists() {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(
                "Failed to restrict permissions for '{}': {}",
                path.display(),
                error
            );
        }
    }
}
