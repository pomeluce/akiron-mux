use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::Db;

#[derive(Debug, Clone, Serialize)]
pub struct BackendDevice {
    pub token_id: String,
    pub name: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

impl Db {
    pub fn insert_backend_device(&self, device: &BackendDevice, digest: &[u8]) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "INSERT INTO backend_devices (token_id, name, token_digest, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![device.token_id, device.name, digest, device.created_at_ms],
        )?;
        Ok(())
    }

    pub fn backend_device_digest(&self, token_id: &str) -> Result<Option<(BackendDevice, Vec<u8>)>, rusqlite::Error> {
        self.conn()
            .query_row(
                "SELECT token_id, name, token_digest, created_at_ms, last_used_at_ms, revoked_at_ms FROM backend_devices WHERE token_id = ?1",
                params![token_id],
                |row| {
                    Ok((
                        BackendDevice {
                            token_id: row.get(0)?,
                            name: row.get(1)?,
                            created_at_ms: row.get(3)?,
                            last_used_at_ms: row.get(4)?,
                            revoked_at_ms: row.get(5)?,
                        },
                        row.get(2)?,
                    ))
                },
            )
            .optional()
    }

    pub fn list_backend_devices(&self) -> Result<Vec<BackendDevice>, rusqlite::Error> {
        let mut statement = self
            .conn()
            .prepare("SELECT token_id, name, created_at_ms, last_used_at_ms, revoked_at_ms FROM backend_devices ORDER BY created_at_ms")?;
        let devices = statement
            .query_map([], |row| {
                Ok(BackendDevice {
                    token_id: row.get(0)?,
                    name: row.get(1)?,
                    created_at_ms: row.get(2)?,
                    last_used_at_ms: row.get(3)?,
                    revoked_at_ms: row.get(4)?,
                })
            })?
            .collect();
        devices
    }

    pub fn touch_backend_device(&self, token_id: &str, now_ms: i64) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "UPDATE backend_devices SET last_used_at_ms = ?2 WHERE token_id = ?1 AND revoked_at_ms IS NULL",
            params![token_id, now_ms],
        )?;
        Ok(())
    }

    pub fn revoke_backend_device(&self, token_id: &str, now_ms: i64) -> Result<bool, rusqlite::Error> {
        Ok(self.conn().execute(
            "UPDATE backend_devices SET revoked_at_ms = ?2 WHERE token_id = ?1 AND revoked_at_ms IS NULL",
            params![token_id, now_ms],
        )? > 0)
    }

    pub fn has_active_backend_device(&self) -> Result<bool, rusqlite::Error> {
        self.conn()
            .query_row("SELECT EXISTS(SELECT 1 FROM backend_devices WHERE revoked_at_ms IS NULL)", [], |row| row.get(0))
    }

    pub fn record_backend_audit(&self, event: &str, device_id: Option<&str>, source: Option<&str>, now_ms: i64) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "INSERT INTO backend_audit (event, device_id, source, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![event, device_id, source, now_ms],
        )?;
        let cutoff = now_ms - 30 * 24 * 60 * 60 * 1000;
        self.conn().execute("DELETE FROM backend_audit WHERE created_at_ms < ?1", params![cutoff])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_lifecycle_never_requires_plaintext_storage() {
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        let device = BackendDevice {
            token_id: "device-1".into(),
            name: "Phone".into(),
            created_at_ms: 10,
            last_used_at_ms: None,
            revoked_at_ms: None,
        };
        db.insert_backend_device(&device, &[1, 2, 3]).unwrap();

        let (stored, digest) = db.backend_device_digest("device-1").unwrap().unwrap();
        assert_eq!(stored.name, "Phone");
        assert_eq!(digest, [1, 2, 3]);
        assert!(db.has_active_backend_device().unwrap());

        assert!(db.revoke_backend_device("device-1", 20).unwrap());
        assert!(!db.has_active_backend_device().unwrap());
    }
}
