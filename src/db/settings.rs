use super::connection::Db;
use rusqlite::params;

impl Db {
    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn().query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0)).ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        self.conn().execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    pub fn set_settings(&self, settings: &[(&str, &str)]) -> Result<(), rusqlite::Error> {
        let transaction = rusqlite::Transaction::new_unchecked(self.conn(), rusqlite::TransactionBehavior::Immediate)?;
        for (key, value) in settings {
            transaction.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![key, value])?;
        }
        transaction.commit()
    }
}
