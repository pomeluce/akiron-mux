use super::connection::Db;
use crate::core::models::{CodexModel, Profile, Provider, Source};
use rusqlite::{params, types::Type, Connection, Transaction, TransactionBehavior};

// ── Providers ──

impl Db {
    pub fn insert_provider(&self, p: &Provider, app_type: &str) -> Result<(), rusqlite::Error> {
        let source_str: &str = p.source.as_str();
        self.conn().execute(
            "INSERT INTO providers (id, app_type, name, api_url, api_key, codex_catalog, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id, app_type) DO UPDATE SET
                name=excluded.name, api_url=excluded.api_url,
                api_key=excluded.api_key, codex_catalog=excluded.codex_catalog,
                source=excluded.source",
            params![p.id, app_type, p.name, p.api_url, p.api_key, p.codex_catalog.as_str(), source_str],
        )?;
        Ok(())
    }

    pub fn get_providers(&self, app_type: &str) -> Result<Vec<Provider>, rusqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, name, api_url, api_key, codex_catalog, source FROM providers WHERE app_type = ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![app_type], |row| {
            let catalog: String = row.get(4)?;
            let source_str: String = row.get(5)?;
            Ok(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                api_url: row.get(2)?,
                api_key: row.get(3)?,
                codex_catalog: catalog
                    .parse()
                    .map_err(|_| rusqlite::Error::FromSqlConversionFailure(4, Type::Text, format!("invalid Codex catalog value: {}", catalog).into()))?,
                profiles: vec![],
                models: vec![],
                source: source_str.parse().unwrap_or(Source::System),
            })
        })?;
        rows.collect()
    }

    pub fn insert_codex_model(&self, provider_id: &str, model: &CodexModel) -> Result<(), rusqlite::Error> {
        let transaction = Transaction::new_unchecked(self.conn(), TransactionBehavior::Immediate)?;
        insert_codex_model(&transaction, provider_id, model)?;
        transaction.commit()
    }

    pub(crate) fn insert_codex_model_in(&self, transaction: &Transaction<'_>, provider_id: &str, model: &CodexModel) -> Result<(), rusqlite::Error> {
        insert_codex_model(transaction, provider_id, model)
    }

    pub(crate) fn clear_codex_model_in(&self, transaction: &Transaction<'_>, provider_id: &str, slug: &str) -> Result<(), rusqlite::Error> {
        transaction.execute("DELETE FROM codex_models WHERE provider_id=?1 AND slug=?2", params![provider_id, slug])?;
        Ok(())
    }

    pub(crate) fn save_provider_in(&self, transaction: &Transaction<'_>, provider: &Provider, app_type: &str) -> Result<(), rusqlite::Error> {
        let source = provider.source.as_str();
        transaction.execute(
            "INSERT INTO providers (id, app_type, name, api_url, api_key, codex_catalog, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id, app_type) DO UPDATE SET
                name=excluded.name, api_url=excluded.api_url,
                api_key=excluded.api_key, codex_catalog=excluded.codex_catalog,
                source=excluded.source",
            params![
                provider.id,
                app_type,
                provider.name,
                provider.api_url,
                provider.api_key,
                provider.codex_catalog.as_str(),
                source
            ],
        )?;
        Ok(())
    }

    pub(crate) fn save_profile_in(&self, transaction: &Transaction<'_>, provider_id: &str, profile: &Profile) -> Result<(), rusqlite::Error> {
        transaction.execute(
            "INSERT INTO profiles (id, name, provider_id, opus_model, sonnet_model, haiku_model, subagent_model, is_default, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id, provider_id) DO UPDATE SET
                name=excluded.name, provider_id=excluded.provider_id,
                opus_model=excluded.opus_model, sonnet_model=excluded.sonnet_model,
                haiku_model=excluded.haiku_model, subagent_model=excluded.subagent_model,
                is_default=excluded.is_default, source=excluded.source",
            params![
                profile.id,
                profile.name,
                provider_id,
                profile.opus,
                profile.sonnet,
                profile.haiku,
                profile.subagent,
                profile.default as i32,
                profile.source.as_str()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn delete_profile_in(&self, transaction: &Transaction<'_>, provider_id: &str, profile_id: &str) -> Result<(), rusqlite::Error> {
        transaction.execute("DELETE FROM profiles WHERE provider_id = ?1 AND id = ?2", params![provider_id, profile_id])?;
        Ok(())
    }

    pub(crate) fn delete_provider_in(&self, transaction: &Transaction<'_>, provider_id: &str, app_type: &str) -> Result<(), rusqlite::Error> {
        if app_type == "claude" {
            transaction.execute("DELETE FROM profiles WHERE provider_id = ?1", params![provider_id])?;
        }
        if app_type == "codex" {
            transaction.execute("DELETE FROM codex_models WHERE provider_id = ?1", params![provider_id])?;
        }
        transaction.execute("DELETE FROM providers WHERE id = ?1 AND app_type = ?2", params![provider_id, app_type])?;
        Ok(())
    }
}

fn insert_codex_model(connection: &Connection, provider_id: &str, model: &CodexModel) -> Result<(), rusqlite::Error> {
    let efforts = serde_json::to_string(&model.supported_reasoning_efforts).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
    let modalities = serde_json::to_string(&model.input_modalities).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
    if model.default {
        connection.execute("UPDATE codex_models SET is_default=0 WHERE provider_id=?1", params![provider_id])?;
    }
    connection.execute(
        "INSERT INTO codex_models
             (provider_id, slug, display_name, description, context_window, max_context_window,
              effective_context_window_percent, default_reasoning_effort,
              supported_reasoning_efforts, input_modalities, supports_parallel_tool_calls,
              support_verbosity, default_verbosity, supports_search_tool, is_default, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(provider_id, slug) DO UPDATE SET
              display_name=excluded.display_name, description=excluded.description,
              context_window=excluded.context_window, max_context_window=excluded.max_context_window,
              effective_context_window_percent=excluded.effective_context_window_percent,
              default_reasoning_effort=excluded.default_reasoning_effort,
              supported_reasoning_efforts=excluded.supported_reasoning_efforts,
              input_modalities=excluded.input_modalities,
              supports_parallel_tool_calls=excluded.supports_parallel_tool_calls,
              support_verbosity=excluded.support_verbosity, default_verbosity=excluded.default_verbosity,
              supports_search_tool=excluded.supports_search_tool, is_default=excluded.is_default,
              source=excluded.source",
        params![
            provider_id,
            model.slug,
            model.display_name,
            model.description,
            model.context_window as i64,
            model.max_context_window.map(|value| value as i64),
            model.effective_context_window_percent as i64,
            model.default_reasoning_effort,
            efforts,
            modalities,
            model.supports_parallel_tool_calls,
            model.support_verbosity,
            model.default_verbosity,
            model.supports_search_tool,
            model.default,
            model.source.as_str()
        ],
    )?;
    Ok(())
}

impl Db {
    pub fn get_codex_models(&self, provider_id: &str) -> Result<Vec<CodexModel>, rusqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT slug, display_name, description, context_window, max_context_window,
             effective_context_window_percent, default_reasoning_effort, supported_reasoning_efforts,
             input_modalities, supports_parallel_tool_calls, support_verbosity, default_verbosity,
             supports_search_tool, is_default, source
             FROM codex_models WHERE provider_id = ?1 ORDER BY is_default DESC, display_name",
        )?;
        let rows = stmt.query_map(params![provider_id], |row| {
            let efforts: String = row.get(7)?;
            let modalities: String = row.get(8)?;
            let source: String = row.get(14)?;
            Ok(CodexModel {
                slug: row.get(0)?,
                display_name: row.get(1)?,
                description: row.get(2)?,
                context_window: row.get::<_, i64>(3)? as u64,
                max_context_window: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                effective_context_window_percent: row.get::<_, i64>(5)? as u8,
                default_reasoning_effort: row.get(6)?,
                supported_reasoning_efforts: serde_json::from_str(&efforts).map_err(|error| rusqlite::Error::FromSqlConversionFailure(7, Type::Text, error.into()))?,
                input_modalities: serde_json::from_str(&modalities).map_err(|error| rusqlite::Error::FromSqlConversionFailure(8, Type::Text, error.into()))?,
                supports_parallel_tool_calls: row.get(9)?,
                support_verbosity: row.get(10)?,
                default_verbosity: row.get(11)?,
                supports_search_tool: row.get(12)?,
                default: row.get(13)?,
                source: source.parse().unwrap_or(Source::User),
            })
        })?;
        rows.collect()
    }

    pub fn delete_codex_model(&self, provider_id: &str, slug: &str) -> Result<(), rusqlite::Error> {
        self.conn()
            .execute("DELETE FROM codex_models WHERE provider_id=?1 AND slug=?2", params![provider_id, slug])?;
        Ok(())
    }

    /// Replace system-defined Codex models while preserving user overrides.
    pub fn sync_system_codex_models(&self, providers: &[Provider]) -> Result<(), rusqlite::Error> {
        let transaction = self.conn().unchecked_transaction()?;
        transaction.execute("DELETE FROM codex_models WHERE source='system'", [])?;
        for provider in providers {
            for model in &provider.models {
                let efforts = serde_json::to_string(&model.supported_reasoning_efforts).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
                let modalities = serde_json::to_string(&model.input_modalities).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
                transaction.execute(
                    "INSERT OR IGNORE INTO codex_models
                     (provider_id, slug, display_name, description, context_window, max_context_window,
                      effective_context_window_percent, default_reasoning_effort,
                      supported_reasoning_efforts, input_modalities, supports_parallel_tool_calls,
                      support_verbosity, default_verbosity, supports_search_tool, is_default, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'system')",
                    params![
                        provider.id,
                        model.slug,
                        model.display_name,
                        model.description,
                        model.context_window as i64,
                        model.max_context_window.map(|value| value as i64),
                        model.effective_context_window_percent as i64,
                        model.default_reasoning_effort,
                        efforts,
                        modalities,
                        model.supports_parallel_tool_calls,
                        model.support_verbosity,
                        model.default_verbosity,
                        model.supports_search_tool,
                        model.default
                    ],
                )?;
            }
        }
        transaction.commit()
    }

    pub fn delete_provider(&self, id: &str, app_type: &str) -> Result<(), rusqlite::Error> {
        // Profiles belong to Claude providers only. A Codex provider may reuse
        // the same id and must not delete Claude profiles.
        if app_type == "claude" {
            self.conn().execute("DELETE FROM profiles WHERE provider_id = ?1", params![id])?;
        }
        if app_type == "codex" {
            self.conn().execute("DELETE FROM codex_models WHERE provider_id = ?1", params![id])?;
        }
        self.conn().execute("DELETE FROM providers WHERE id = ?1 AND app_type = ?2", params![id, app_type])?;
        Ok(())
    }

    /// Sync system providers/profiles from defaults.toml into the DB.
    /// - New TOML providers → INSERT with source='system'
    /// - Existing system providers → UPDATE fields from TOML
    /// - User-added providers (source='user') → never touched
    /// - DB providers with source='system' not in TOML → demote to source='user'
    pub fn sync_system_providers(&self, app_type: &str, system_providers: &[Provider]) -> Result<(), rusqlite::Error> {
        let transaction = self.conn().unchecked_transaction()?;
        let mut toml_ids: Vec<&str> = Vec::new();
        let mut toml_profile_keys: Vec<(String, String)> = Vec::new();
        for p in system_providers {
            toml_ids.push(&p.id);
            // INSERT only if not already present (user row takes priority)
            transaction.execute(
                "INSERT OR IGNORE INTO providers
                 (id, app_type, name, api_url, api_key, codex_catalog, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'system')",
                params![p.id, app_type, p.name, p.api_url, p.api_key, p.codex_catalog.as_str()],
            )?;
            // UPDATE existing system providers with latest TOML values
            transaction.execute(
                "UPDATE providers SET name=?1, api_url=?2, api_key=?3,
                 codex_catalog=?4, source='system'
                 WHERE id=?5 AND app_type=?6 AND source='system'",
                params![p.name, p.api_url, p.api_key, p.codex_catalog.as_str(), p.id, app_type],
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
            let placeholders: Vec<String> = toml_ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect();
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
            transaction.execute(sql, rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())))?;
        }
        if app_type == "claude" {
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(app_type.to_string())];
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
                        format!("(provider_id=?{} AND id=?{})", provider_param, profile_param)
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
            transaction.execute(&sql, rusqlite::params_from_iter(params.iter().map(|param| param.as_ref())))?;
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
        self.conn().execute("DELETE FROM profiles WHERE provider_id = ?1 AND id = ?2", params![provider_id, id])?;
        Ok(())
    }
}
