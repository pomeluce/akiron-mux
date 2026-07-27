use super::connection::Db;
use crate::core::models::{Profile, Provider, Source};
use rusqlite::params;

// ── Providers ──

impl Db {
    pub fn insert_provider(&self, p: &Provider, app_type: &str) -> Result<(), rusqlite::Error> {
        let source_str: &str = p.source.as_str();
        self.conn().execute(
            "INSERT INTO providers (id, app_type, name, api_url, api_key, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id, app_type) DO UPDATE SET
                name=excluded.name, api_url=excluded.api_url,
                api_key=excluded.api_key, source=excluded.source",
            params![p.id, app_type, p.name, p.api_url, p.api_key, source_str],
        )?;
        Ok(())
    }

    pub fn get_providers(&self, app_type: &str) -> Result<Vec<Provider>, rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT id, name, api_url, api_key, source FROM providers WHERE app_type = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![app_type], |row| {
            let source_str: String = row.get(4)?;
            Ok(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                api_url: row.get(2)?,
                api_key: row.get(3)?,
                profiles: vec![],
                source: source_str.parse().unwrap_or(Source::System),
            })
        })?;
        rows.collect()
    }

    pub fn delete_provider(&self, id: &str, app_type: &str) -> Result<(), rusqlite::Error> {
        // Profiles belong to Claude providers only. A Codex provider may reuse
        // the same id and must not delete Claude profiles.
        if app_type == "claude" {
            self.conn()
                .execute("DELETE FROM profiles WHERE provider_id = ?1", params![id])?;
        }
        self.conn().execute(
            "DELETE FROM providers WHERE id = ?1 AND app_type = ?2",
            params![id, app_type],
        )?;
        Ok(())
    }

    /// Sync system providers/profiles from defaults.toml into the DB.
    /// - New TOML providers → INSERT with source='system'
    /// - Existing system providers → UPDATE fields from TOML
    /// - User-added providers (source='user') → never touched
    /// - DB providers with source='system' not in TOML → demote to source='user'
    pub fn sync_system_providers(
        &self,
        app_type: &str,
        system_providers: &[Provider],
    ) -> Result<(), rusqlite::Error> {
        let transaction = self.conn().unchecked_transaction()?;
        let mut toml_ids: Vec<&str> = Vec::new();
        let mut toml_profile_keys: Vec<(String, String)> = Vec::new();
        for p in system_providers {
            toml_ids.push(&p.id);
            // INSERT only if not already present (user row takes priority)
            transaction.execute(
                "INSERT OR IGNORE INTO providers (id, app_type, name, api_url, api_key, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'system')",
                params![p.id, app_type, p.name, p.api_url, p.api_key],
            )?;
            // UPDATE existing system providers with latest TOML values
            transaction.execute(
                "UPDATE providers SET name=?1, api_url=?2, api_key=?3, source='system'
                 WHERE id=?4 AND app_type=?5 AND source='system'",
                params![p.name, p.api_url, p.api_key, p.id, app_type],
            )?;
            // Sync profiles: INSERT OR IGNORE for system ones
            for pr in &p.profiles {
                toml_profile_keys.push((p.id.clone(), pr.id.clone()));
                transaction.execute(
                    "INSERT INTO profiles (id, name, provider_id, opus_model, sonnet_model, haiku_model, subagent_model, is_default, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'system')
                     ON CONFLICT(id, provider_id) DO UPDATE SET
                        name=excluded.name, provider_id=excluded.provider_id,
                        opus_model=excluded.opus_model, sonnet_model=excluded.sonnet_model,
                        haiku_model=excluded.haiku_model, subagent_model=excluded.subagent_model,
                        is_default=excluded.is_default
                     WHERE profiles.source='system'",
                    params![pr.id, pr.name, p.id, pr.opus, pr.sonnet, pr.haiku, pr.subagent, pr.default as i32],
                )?;
            }
        }
        // Demote system providers that no longer exist in TOML (always run —
        // even when toml is empty, to clean up providers removed from defaults)
        {
            let placeholders: Vec<String> = toml_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect();
            let sql = if toml_ids.is_empty() {
                "UPDATE providers SET source = 'user' WHERE app_type = ?1 AND source = 'system'"
            } else {
                &format!(
                    "UPDATE providers SET source = 'user' WHERE app_type = ?1 AND source = 'system' AND id NOT IN ({})",
                    placeholders.join(",")
                )
            };
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(app_type.to_string()));
            for id in &toml_ids {
                param_values.push(Box::new(id.to_string()));
            }
            transaction.execute(
                sql,
                rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())),
            )?;
        }
        if app_type == "claude" {
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(app_type.to_string())];
            let sql = if toml_profile_keys.is_empty() {
                "UPDATE profiles SET source='user'
                 WHERE source='system'
                   AND provider_id IN (SELECT id FROM providers WHERE app_type=?1)"
                    .to_string()
            } else {
                let clauses = toml_profile_keys
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        let provider_param = index * 2 + 2;
                        let profile_param = provider_param + 1;
                        format!(
                            "(provider_id=?{} AND id=?{})",
                            provider_param, profile_param
                        )
                    })
                    .collect::<Vec<_>>();
                for (provider_id, profile_id) in &toml_profile_keys {
                    params.push(Box::new(provider_id.clone()));
                    params.push(Box::new(profile_id.clone()));
                }
                format!(
                    "UPDATE profiles SET source='user'
                     WHERE source='system'
                       AND provider_id IN (SELECT id FROM providers WHERE app_type=?1)
                       AND NOT ({})",
                    clauses.join(" OR ")
                )
            };
            transaction.execute(
                &sql,
                rusqlite::params_from_iter(params.iter().map(|param| param.as_ref())),
            )?;
        }
        transaction.commit()
    }
}

// ── Claude profiles ──

impl Db {
    pub fn insert_profile(&self, provider_id: &str, p: &Profile) -> Result<(), rusqlite::Error> {
        let source_str: &str = p.source.as_str();
        self.conn().execute(
            "INSERT INTO profiles (id, name, provider_id, opus_model, sonnet_model, haiku_model, subagent_model, is_default, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id, provider_id) DO UPDATE SET
                name=excluded.name, provider_id=excluded.provider_id,
                opus_model=excluded.opus_model, sonnet_model=excluded.sonnet_model,
                haiku_model=excluded.haiku_model, subagent_model=excluded.subagent_model,
                is_default=excluded.is_default, source=excluded.source",
            params![p.id, p.name, provider_id, p.opus, p.sonnet, p.haiku, p.subagent, p.default as i32, source_str],
        )?;
        Ok(())
    }

    pub fn get_profiles(&self, provider_id: &str) -> Result<Vec<Profile>, rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT id, name, opus_model, sonnet_model, haiku_model, subagent_model, is_default, source
             FROM profiles WHERE provider_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![provider_id], |row| {
            let source_str: String = row.get(7)?;
            Ok(Profile {
                id: row.get(0)?,
                name: row.get(1)?,
                opus: row.get(2)?,
                sonnet: row.get(3)?,
                haiku: row.get(4)?,
                subagent: row.get(5)?,
                default: row.get::<_, i32>(6)? != 0,
                source: source_str.parse().unwrap_or(Source::System),
            })
        })?;
        rows.collect()
    }

    pub fn delete_profile(&self, provider_id: &str, id: &str) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "DELETE FROM profiles WHERE provider_id = ?1 AND id = ?2",
            params![provider_id, id],
        )?;
        Ok(())
    }
}
