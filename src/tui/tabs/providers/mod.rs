pub mod form;

use super::super::theme;
use super::super::widgets::shared::{
    clear_popup_area, render_confirm_popup as shared_confirm, render_message_popup as shared_msg,
};
use super::TabContent;
use crate::core::codex_catalog::{
    catalog_status, default_catalog_path, model_entry, write_catalog,
};
use crate::core::config::ConfigManager;
use crate::core::models::{
    validate_codex_model, validate_profile, validate_provider, AppType, CodexCatalog, CodexModel,
    Profile, Provider,
};
use crate::tui::lang;
use crossterm::event::KeyCode;
use form::{CodexModelForm, EditForm, ProviderForm};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    ProviderList,
    ProfileList,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProviderAction {
    Switch,
    Delete,
}

struct ContentPopup {
    title: String,
    content: String,
    compact: bool,
    scroll: u16,
    max_scroll: u16,
    page_height: u16,
}

pub struct ProvidersTab {
    mgr: Rc<ConfigManager>,
    app: AppType,
    // Provider list
    providers: Vec<Provider>,
    provider_state: ListState,
    selected_provider_idx: usize,
    // Profile list
    profiles: Vec<Profile>,
    codex_models: Vec<CodexModel>,
    profile_state: ListState,
    selected_profile_idx: usize,
    // Active state
    pub active_provider: String,
    pub active_profile: String,
    pub active_codex_model: String,
    // Navigation
    panel: Panel,
    // Search
    pub search_query: String,
    pub is_searching: bool,
    // Popups
    pub confirm_action: Option<ProviderAction>,
    confirm_button: usize,
    pub message: Option<String>,
    content_popup: Option<ContentPopup>,
    status_message: Option<String>,
    edit_form: Option<EditForm>,
    provider_form: Option<ProviderForm>,
    codex_model_form: Option<CodexModelForm>,
}

impl ProvidersTab {
    pub fn new(mgr: Rc<ConfigManager>, app: AppType) -> Self {
        crate::core::sync::sync_active_from_settings(&mgr);
        crate::core::sync::sync_codex_active_from_config(&mgr);

        let mut providers = mgr.list_providers_for(app).unwrap_or_default();
        providers.sort_by(
            |a, b| match (a.source.can_delete(), b.source.can_delete()) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            },
        );
        let active_provider = mgr
            .get_setting(app.active_provider_key())
            .unwrap_or_default();
        let active_profile = if app == AppType::Claude {
            mgr.get_setting("active_profile").unwrap_or_default()
        } else {
            String::new()
        };
        let active_codex_model = mgr.get_setting("active_codex_model").unwrap_or_default();

        let selected_provider_idx = providers
            .iter()
            .position(|p| p.id == active_provider)
            .unwrap_or(0);

        let profiles = if let Some(p) = providers.get(selected_provider_idx) {
            p.profiles.clone()
        } else {
            vec![]
        };
        let codex_models = providers
            .get(selected_provider_idx)
            .map(|provider| provider.models.clone())
            .unwrap_or_default();

        let selected_profile_idx = if app == AppType::Codex {
            codex_models
                .iter()
                .position(|model| model.slug == active_codex_model)
                .unwrap_or(0)
        } else if profiles.is_empty() {
            0
        } else {
            profiles
                .iter()
                .position(|pr| pr.id == active_profile)
                .unwrap_or(0)
        };

        let mut provider_state = ListState::default();
        provider_state.select(Some(selected_provider_idx));
        let mut profile_state = ListState::default();
        profile_state.select(if profiles.is_empty() && codex_models.is_empty() {
            None
        } else {
            Some(selected_profile_idx)
        });

        ProvidersTab {
            mgr,
            app,
            providers,
            provider_state,
            selected_provider_idx,
            profiles,
            codex_models,
            profile_state,
            selected_profile_idx,
            active_provider,
            active_profile,
            active_codex_model,
            panel: Panel::ProviderList,
            search_query: String::new(),
            is_searching: false,
            confirm_action: None,
            confirm_button: 0,
            message: None,
            content_popup: None,
            status_message: None,
            edit_form: None,
            provider_form: None,
            codex_model_form: None,
        }
    }

    pub fn switch_app(&mut self, app: AppType) {
        self.app = app;
        self.panel = Panel::ProviderList;
        self.confirm_action = None;
        self.message = None;
        self.content_popup = None;
        self.status_message = None;
        self.edit_form = None;
        self.provider_form = None;
        self.codex_model_form = None;
        self.selected_provider_idx = 0;
        self.selected_profile_idx = 0;
        self.active_provider = self
            .mgr
            .get_setting(app.active_provider_key())
            .unwrap_or_default();
        self.active_profile = if app == AppType::Claude {
            self.mgr.get_setting("active_profile").unwrap_or_default()
        } else {
            String::new()
        };
        self.active_codex_model = self
            .mgr
            .get_setting("active_codex_model")
            .unwrap_or_default();
        self.refresh_providers();
        if let Some(index) = self
            .providers
            .iter()
            .position(|p| p.id == self.active_provider)
        {
            self.selected_provider_idx = index;
            self.provider_state.select(Some(index));
            self.load_profiles();
        }
    }

    pub fn active_context(&self) -> String {
        if self.active_provider.is_empty() {
            String::new()
        } else if self.app == AppType::Claude && !self.active_profile.is_empty() {
            format!("{}/{}", self.active_provider, self.active_profile)
        } else {
            if self.app == AppType::Codex && !self.active_codex_model.is_empty() {
                format!("{}/{}", self.active_provider, self.active_codex_model)
            } else {
                self.active_provider.clone()
            }
        }
    }

    pub fn status_text(&self) -> String {
        if let Some(message) = &self.status_message {
            return message.clone();
        }
        let provider = self
            .selected_provider()
            .map(|p| p.name.as_str())
            .unwrap_or("-");
        if self.app == AppType::Claude {
            let profile = self
                .selected_profile()
                .map(|p| p.name.as_str())
                .unwrap_or("-");
            format!("Claude · {} / {}", provider, profile)
        } else {
            format!("Codex · {}", provider)
        }
    }

    // ── Provider CRUD ──

    fn do_add_provider(&mut self) {
        self.provider_form = Some(ProviderForm {
            fields: [String::new(), String::new(), String::new(), String::new()],
            cursors: [0, 0, 0, 0],
            focused: 0,
            is_edit: false,
            show_catalog: self.app == AppType::Codex,
            custom_catalog: self.app == AppType::Codex,
        });
    }

    fn do_edit_provider(&mut self) {
        let Some(prov) = self.selected_provider() else {
            return;
        };
        if !prov.source.can_delete() {
            self.message = Some(lang::current().msg_cannot_edit_sys_provider.into());
            return;
        }
        self.provider_form = Some(ProviderForm {
            fields: [
                prov.name.clone(),
                prov.id.clone(),
                prov.api_url.clone(),
                prov.api_key.clone(),
            ],
            cursors: [
                prov.name.len(),
                prov.id.len(),
                prov.api_url.len(),
                prov.api_key.len(),
            ],
            focused: 0,
            is_edit: true,
            show_catalog: self.app == AppType::Codex,
            custom_catalog: prov.codex_catalog == CodexCatalog::Custom,
        });
    }

    fn commit_provider(&mut self) {
        let Some(form) = self.provider_form.as_ref() else {
            return;
        };
        let pr = Provider {
            id: form.fields[1].clone(),
            name: form.fields[0].clone(),
            api_url: form.fields[2].clone(),
            api_key: form.fields[3].clone(),
            codex_catalog: if self.app == AppType::Codex && form.custom_catalog {
                CodexCatalog::Custom
            } else {
                CodexCatalog::BuiltIn
            },
            profiles: vec![],
            models: vec![],
            source: crate::core::models::Source::User,
        };
        if let Err(error) = validate_provider(&pr) {
            self.message = Some(localized_error(&error));
            return;
        }
        if !form.is_edit && self.providers.iter().any(|provider| provider.id == pr.id) {
            self.message = Some(lang::pick_owned(
                format!("Provider '{}' already exists", pr.id),
                format!("供应商 '{}' 已存在", pr.id),
            ));
            return;
        }
        if form.is_edit
            && self.app == AppType::Codex
            && self.active_provider == pr.id
            && self
                .providers
                .iter()
                .find(|provider| provider.id == pr.id)
                .is_some_and(|provider| provider.codex_catalog != pr.codex_catalog)
        {
            self.message = Some(lang::pick_owned(
                "Switch to another Codex provider before changing the active provider's catalog type"
                    .into(),
                "请先切换到其他 Codex 供应商，再修改当前供应商的模型来源".into(),
            ));
            return;
        }
        if self.app == AppType::Codex {
            let mut prospective = self.providers.clone();
            if let Some(existing) = prospective.iter_mut().find(|provider| provider.id == pr.id) {
                existing.name.clone_from(&pr.name);
                existing.api_url.clone_from(&pr.api_url);
                existing.api_key.clone_from(&pr.api_key);
                existing.codex_catalog = pr.codex_catalog;
            } else {
                prospective.push(pr.clone());
            }
            if let Err(error) = crate::core::codex_catalog::build_catalog(&prospective) {
                self.message = Some(localized_error(&error));
                return;
            }
        }
        let apply_active_codex = self.app == AppType::Codex && self.active_provider == pr.id;
        if let Err(e) = self.mgr.db().insert_provider(&pr, self.app.as_str()) {
            self.message = Some(lang::pick_owned(
                format!("Failed to save provider: {}", e),
                format!("保存供应商失败：{}", e),
            ));
            return;
        }
        self.provider_form = None;
        self.refresh_providers();
        if self.app == AppType::Codex {
            self.rebuild_catalog_if_present();
        }
        if apply_active_codex {
            let model = if self.active_codex_model.is_empty() {
                None
            } else {
                Some(self.active_codex_model.as_str())
            };
            if let Err(error) =
                crate::core::switcher::switch_codex_model(&self.mgr, &pr.id, model, None, None)
            {
                self.message = Some(lang::pick_owned(
                    format!("Provider saved, but applying it to Codex failed: {}", error),
                    format!("供应商已保存，但应用到 Codex 失败：{}", error),
                ));
                return;
            }
        }
        self.status_message = Some(format!("Provider '{}' saved", pr.name));
    }

    fn do_delete_provider(&mut self) {
        let Some(prov) = self.selected_provider() else {
            return;
        };
        if !prov.source.can_delete() {
            self.message = Some(lang::current().msg_cannot_delete_sys_provider.into());
            return;
        }
        let provider_id = prov.id.clone();
        if self.app == AppType::Codex && self.active_provider == provider_id {
            self.message = Some(lang::pick_owned(
                "Switch to another Codex provider before deleting the active provider".into(),
                "请先切换到其他 Codex 供应商，再删除当前供应商".into(),
            ));
            return;
        }
        if self.app == AppType::Codex {
            if let Err(e) =
                crate::core::switcher::remove_codex_provider(&self.mgr, &provider_id, None)
            {
                self.message = Some(lang::pick_owned(
                    format!("Failed to update Codex config.toml: {}", e),
                    format!("更新 Codex config.toml 失败：{}", e),
                ));
                return;
            }
        }
        if let Err(e) = self
            .mgr
            .db()
            .delete_provider(&provider_id, self.app.as_str())
        {
            self.message = Some(lang::pick_owned(
                format!("Failed to delete provider: {}", e),
                format!("删除供应商失败：{}", e),
            ));
            return;
        }
        if self.active_provider == provider_id {
            self.active_provider.clear();
            self.active_profile.clear();
            let _ = self.mgr.set_setting(self.app.active_provider_key(), "");
            if self.app == AppType::Claude {
                let _ = self.mgr.set_setting("active_profile", "");
            }
        }
        self.refresh_providers();
        if self.app == AppType::Codex {
            self.rebuild_catalog_if_present();
        }
        self.status_message = Some(format!("Provider '{}' deleted", provider_id));
    }

    /// Full refresh: re-fetch providers from DB (expensive, call on mutations or Enter)
    fn refresh_providers(&mut self) {
        let mut providers = self.mgr.list_providers_for(self.app).unwrap_or_default();
        providers.sort_by(
            |a, b| match (a.source.can_delete(), b.source.can_delete()) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            },
        );
        self.providers = providers;
        if self.selected_provider_idx >= self.providers.len() {
            self.selected_provider_idx = 0;
        }
        self.load_profiles();
    }

    /// Lightweight: load profiles for selected provider from cached data (no DB call)
    fn load_profiles(&mut self) {
        self.profiles = if self.app == AppType::Claude {
            if let Some(p) = self.providers.get(self.selected_provider_idx) {
                p.profiles.clone()
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        self.codex_models = if self.app == AppType::Codex {
            self.providers
                .get(self.selected_provider_idx)
                .map(|p| p.models.clone())
                .unwrap_or_default()
        } else {
            vec![]
        };
        let selection_len = if self.app == AppType::Codex {
            self.codex_models.len()
        } else {
            self.profiles.len()
        };
        if self.selected_profile_idx >= selection_len {
            self.selected_profile_idx = 0;
        }
        self.profile_state.select(if selection_len == 0 {
            None
        } else {
            Some(self.selected_profile_idx)
        });
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.profiles.get(self.selected_profile_idx)
    }

    fn selected_provider(&self) -> Option<&Provider> {
        self.providers.get(self.selected_provider_idx)
    }

    fn selected_codex_model(&self) -> Option<&CodexModel> {
        self.codex_models.get(self.selected_profile_idx)
    }

    fn do_add_codex_model(&mut self) {
        let Some(provider) = self.selected_provider() else {
            return;
        };
        if provider.codex_catalog != CodexCatalog::Custom {
            self.message = Some(lang::pick_owned(
                "Change this provider to Third-party models before adding a model".into(),
                "请先将该供应商的模型来源改为第三方模型".into(),
            ));
            return;
        }
        self.codex_model_form = Some(CodexModelForm {
            fields: [
                String::new(),
                String::new(),
                String::new(),
                "128000".into(),
                String::new(),
                "95".into(),
            ],
            cursors: [0, 0, 0, 6, 0, 2],
            focused: 0,
            is_edit: false,
            provider_id: provider.id.clone(),
            supported_efforts: [false, false, true, true, true, false, false, false],
            effort_cursor: 3,
            default_effort: 3,
            default_model: provider.models.is_empty(),
            supports_images: false,
            supports_parallel_tools: true,
            support_verbosity: true,
            supports_search: false,
        });
    }

    fn do_edit_codex_model(&mut self) {
        let Some(model) = self.selected_codex_model().cloned() else {
            return;
        };
        if !model.source.can_delete() {
            self.message = Some(lang::current().msg_cannot_edit_sys_profile.into());
            return;
        }
        let fields = [
            model.slug.clone(),
            model.display_name.clone(),
            model.description.clone(),
            model.context_window.to_string(),
            model
                .max_context_window
                .map(|value| value.to_string())
                .unwrap_or_default(),
            model.effective_context_window_percent.to_string(),
        ];
        let cursors = std::array::from_fn(|index| fields[index].len());
        let supported_efforts = std::array::from_fn(|index| {
            model
                .supported_reasoning_efforts
                .iter()
                .any(|value| value == form::REASONING_EFFORTS[index])
        });
        let default_effort = form::REASONING_EFFORTS
            .iter()
            .position(|value| *value == model.default_reasoning_effort)
            .unwrap_or(3);
        let provider_id = self
            .selected_provider()
            .map(|provider| provider.id.clone())
            .unwrap_or_default();
        self.codex_model_form = Some(CodexModelForm {
            fields,
            cursors,
            focused: 0,
            is_edit: true,
            provider_id,
            supported_efforts,
            effort_cursor: default_effort,
            default_effort,
            default_model: model.default,
            supports_images: model.input_modalities.iter().any(|item| item == "image"),
            supports_parallel_tools: model.supports_parallel_tool_calls,
            support_verbosity: model.support_verbosity,
            supports_search: model.supports_search_tool,
        });
    }

    fn commit_codex_model(&mut self) {
        let Some(form) = self.codex_model_form.as_ref() else {
            return;
        };
        let provider_id = form.provider_id.clone();
        let max_context_window = if form.fields[4].trim().is_empty() {
            None
        } else {
            match form.fields[4].trim().parse() {
                Ok(value) => Some(value),
                Err(_) => {
                    self.message = Some(lang::pick_owned(
                        "Maximum context window must be a positive integer".into(),
                        "最大上下文必须是正整数".into(),
                    ));
                    return;
                }
            }
        };
        let model = CodexModel {
            slug: form.fields[0].trim().into(),
            display_name: form.fields[1].trim().into(),
            description: form.fields[2].trim().into(),
            context_window: form.fields[3].trim().parse().unwrap_or(0),
            max_context_window,
            effective_context_window_percent: form.fields[5].trim().parse().unwrap_or(0),
            default_reasoning_effort: form.default_reasoning_effort(),
            supported_reasoning_efforts: form.supported_reasoning_efforts(),
            input_modalities: if form.supports_images {
                vec!["text".into(), "image".into()]
            } else {
                vec!["text".into()]
            },
            supports_parallel_tool_calls: form.supports_parallel_tools,
            support_verbosity: form.support_verbosity,
            default_verbosity: "low".into(),
            supports_search_tool: form.supports_search,
            default: form.default_model,
            source: crate::core::models::Source::User,
        };
        if let Err(error) = validate_codex_model(&model) {
            self.message = Some(localized_error(&error));
            return;
        }
        if !form.is_edit && self.codex_models.iter().any(|item| item.slug == model.slug) {
            self.message = Some(format!("Model '{}' already exists", model.slug));
            return;
        }
        let mut prospective = self.providers.clone();
        if let Some(provider) = prospective
            .iter_mut()
            .find(|provider| provider.id == provider_id)
        {
            if model.default {
                for existing in &mut provider.models {
                    existing.default = false;
                }
            }
            if let Some(existing) = provider
                .models
                .iter_mut()
                .find(|existing| existing.slug == model.slug)
            {
                *existing = model.clone();
            } else {
                provider.models.push(model.clone());
            }
        }
        if let Err(error) = crate::core::codex_catalog::build_catalog(&prospective) {
            self.message = Some(localized_error(&error));
            return;
        }
        if let Err(error) = self.mgr.db().insert_codex_model(&provider_id, &model) {
            self.message = Some(format!("Failed to save model: {}", error));
            return;
        }
        self.codex_model_form = None;
        self.refresh_providers();
        self.rebuild_catalog_if_present();
        if self.active_provider == provider_id && self.active_codex_model == model.slug {
            if let Err(error) = crate::core::switcher::switch_codex_model(
                &self.mgr,
                &provider_id,
                Some(&model.slug),
                None,
                None,
            ) {
                self.message = Some(lang::pick_owned(
                    format!("Model saved, but applying it to Codex failed: {}", error),
                    format!("模型已保存，但应用到 Codex 失败：{}", error),
                ));
            }
        }
        self.status_message = Some(format!("Model '{}' saved", model.display_name));
    }

    fn do_delete_codex_model(&mut self) {
        let Some(model) = self.selected_codex_model().cloned() else {
            return;
        };
        let provider_id = self
            .selected_provider()
            .map(|provider| provider.id.clone())
            .unwrap_or_default();
        if self.active_provider == provider_id && self.active_codex_model == model.slug {
            self.message = Some(lang::pick_owned(
                "Switch to another Codex model before deleting the active model".into(),
                "请先切换到其他 Codex 模型，再删除当前模型".into(),
            ));
            return;
        }
        if let Err(error) = self.mgr.db().delete_codex_model(&provider_id, &model.slug) {
            self.message = Some(format!("Failed to delete model: {}", error));
            return;
        }
        self.refresh_providers();
        self.rebuild_catalog_if_present();
        self.status_message = Some(format!("Model '{}' deleted", model.slug));
    }

    fn rebuild_catalog_if_present(&mut self) {
        let path = default_catalog_path();
        if !path.exists() {
            return;
        }
        let result = self
            .mgr
            .list_providers_for(AppType::Codex)
            .and_then(|providers| write_catalog(&path, &providers));
        if let Err(error) = result {
            self.message = Some(format!("Failed to rebuild models.json: {}", error));
        }
    }

    fn show_catalog_status(&mut self) {
        let path = default_catalog_path();
        let providers = match self.mgr.list_providers_for(AppType::Codex) {
            Ok(providers) => providers,
            Err(error) => {
                self.message = Some(error.to_string());
                return;
            }
        };
        let custom = providers
            .iter()
            .filter(|provider| provider.codex_catalog == CodexCatalog::Custom)
            .collect::<Vec<_>>();
        let models = custom
            .iter()
            .map(|provider| provider.models.len())
            .sum::<usize>();
        let status = match catalog_status(&path, &providers) {
            Ok(true) => lang::pick("Synchronized", "已同步"),
            Ok(false) if path.exists() => lang::pick("Pending", "待同步"),
            Ok(false) => lang::pick("Not generated", "未生成"),
            Err(_) => lang::pick("Invalid", "无效"),
        };
        self.content_popup = Some(ContentPopup {
            title: lang::pick_owned("Model Catalog".into(), "模型目录".into()),
            content: format!(
                "{}: {}\n{}: {}\n{}: {}\n{}: {}",
                lang::pick("File", "文件"),
                path.display(),
                lang::pick("Providers", "供应商"),
                custom.len(),
                lang::pick("Models", "模型"),
                models,
                lang::pick("Status", "状态"),
                status
            ),
            compact: true,
            scroll: 0,
            max_scroll: 0,
            page_height: 1,
        });
    }

    fn preview_codex_model(&mut self) {
        if let Some(model) = self.selected_codex_model() {
            self.content_popup = Some(ContentPopup {
                title: lang::pick_owned("Model JSON Preview".into(), "模型 JSON 预览".into()),
                content: serde_json::to_string_pretty(&model_entry(model))
                    .unwrap_or_else(|error| error.to_string()),
                compact: false,
                scroll: 0,
                max_scroll: 0,
                page_height: 1,
            });
        }
    }

    fn do_add_profile(&mut self) {
        if self.app == AppType::Codex {
            return;
        }
        let prov_id = self
            .selected_provider()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        if prov_id.is_empty() {
            return;
        }
        self.edit_form = Some(EditForm {
            fields: std::array::from_fn(|_| String::new()),
            cursors: [0; 6],
            focused: 0,
            is_edit: false,
            prov_id,
        });
    }

    fn do_edit(&mut self) {
        let Some(prof) = self.selected_profile() else {
            return;
        };
        let fields = [
            prof.id.clone(),
            prof.name.clone(),
            prof.opus.clone(),
            prof.sonnet.clone(),
            prof.haiku.clone(),
            prof.subagent.clone(),
        ];
        let cursors = std::array::from_fn(|index| fields[index].len());
        let prov_id = self
            .selected_provider()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        self.edit_form = Some(EditForm {
            fields,
            cursors,
            focused: 0,
            is_edit: true,
            prov_id,
        });
    }

    fn commit_edit(&mut self) {
        let Some(form) = self.edit_form.as_ref() else {
            return;
        };
        let prof_id = if form.fields[0].is_empty() {
            form.fields[1].to_lowercase().replace(' ', "-")
        } else {
            form.fields[0].clone()
        };
        let pr = Profile {
            id: prof_id,
            name: form.fields[1].clone(),
            opus: form.fields[2].clone(),
            sonnet: form.fields[3].clone(),
            haiku: form.fields[4].clone(),
            subagent: form.fields[5].clone(),
            default: false,
            source: crate::core::models::Source::User,
        };
        if let Err(error) = validate_profile(&pr) {
            self.message = Some(localized_error(&error));
            return;
        }
        if !form.is_edit
            && self
                .providers
                .iter()
                .find(|provider| provider.id == form.prov_id)
                .is_some_and(|provider| provider.profiles.iter().any(|profile| profile.id == pr.id))
        {
            self.message = Some(lang::pick_owned(
                format!("Profile '{}/{}' already exists", form.prov_id, pr.id),
                format!("模型配置 '{}/{}' 已存在", form.prov_id, pr.id),
            ));
            return;
        }
        if let Err(e) = self.mgr.db().insert_profile(&form.prov_id, &pr) {
            self.message = Some(lang::pick_owned(
                format!("Failed to save profile: {}", e),
                format!("保存模型配置失败：{}", e),
            ));
            tracing::error!("Failed to insert user profile: {}", e);
            return;
        }
        self.edit_form = None;
        self.refresh_providers();
        self.status_message = Some(format!("Profile '{}' saved", pr.name));
    }

    fn do_switch(&mut self) {
        let prov_id = match self.selected_provider() {
            Some(p) => p.id.clone(),
            None => return,
        };
        if self.app == AppType::Codex {
            let model_slug = self.selected_codex_model().map(|model| model.slug.clone());
            match crate::core::switcher::switch_codex_model(
                &self.mgr,
                &prov_id,
                model_slug.as_deref(),
                None,
                None,
            ) {
                Ok(_) => {
                    self.active_provider = prov_id.clone();
                    self.active_codex_model = model_slug.unwrap_or_default();
                    self.status_message =
                        Some(format!("Codex switched to '{}'", self.active_context()));
                }
                Err(e) => self.message = Some(localized_error(&e)),
            }
            return;
        }
        let prof_id = match self.selected_profile() {
            Some(profile) => profile.id.clone(),
            None => return,
        };
        let mode = if self
            .mgr
            .get_setting("proxy_mode")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            crate::core::models::SwitchMode::Proxy
        } else {
            crate::core::models::SwitchMode::Local
        };
        if let Err(e) =
            crate::core::switcher::switch_profile(&self.mgr, &prov_id, &prof_id, mode, None)
        {
            self.message = Some(localized_error(&e));
            return;
        }
        self.active_provider = prov_id;
        self.active_profile = prof_id;
        self.status_message = Some(format!(
            "Claude switched to '{}/{}'",
            self.active_provider, self.active_profile
        ));
        if let Err(e) = self
            .mgr
            .set_setting("active_provider", &self.active_provider)
        {
            tracing::error!("Failed to save active_provider: {}", e);
        }
        if let Err(e) = self.mgr.set_setting("active_profile", &self.active_profile) {
            tracing::error!("Failed to save active_profile: {}", e);
        }
    }

    fn do_delete(&mut self) {
        let (provider_id, prof_id) = {
            let Some(prof) = self.selected_profile() else {
                return;
            };
            if !prof.source.can_delete() {
                return;
            }
            let provider_id = self
                .selected_provider()
                .map(|provider| provider.id.clone())
                .unwrap_or_default();
            (provider_id, prof.id.clone())
        };
        if let Err(e) = self.mgr.db().delete_profile(&provider_id, &prof_id) {
            self.message = Some(lang::pick_owned(
                format!("Failed to delete profile: {}", e),
                format!("删除模型配置失败：{}", e),
            ));
            tracing::error!("Failed to delete user profile: {}", e);
            return;
        }
        if self.active_provider == provider_id && self.active_profile == prof_id {
            self.active_profile.clear();
            let _ = self.mgr.set_setting("active_profile", "");
        }
        self.refresh_providers();
        self.status_message = Some(format!("Profile '{}' deleted", prof_id));
    }

    fn render_edit_form(&self, f: &mut Frame, area: Rect) {
        if let Some(ref form) = self.edit_form {
            form::render_edit_form(form, f, area);
        }
    }

    fn render_provider_form(&self, f: &mut Frame, area: Rect) {
        if let Some(ref form) = self.provider_form {
            form::render_provider_form(form, self.app == AppType::Codex, f, area);
        }
    }

    fn render_codex_model_form(&self, f: &mut Frame, area: Rect) {
        if let Some(ref form) = self.codex_model_form {
            form::render_codex_model_form(form, f, area);
        }
    }

    fn render_confirm_popup(&self, f: &mut Frame, area: Rect) {
        let (title, msg, c) = match self.confirm_action {
            Some(ProviderAction::Delete) => {
                if self.panel == Panel::ProviderList {
                    (
                        lang::current().confirm_delete_provider,
                        lang::current().confirm_delete_provider_msg,
                        theme::current().red,
                    )
                } else {
                    (
                        lang::current().confirm_delete_profile,
                        lang::current().confirm_delete_profile_msg,
                        theme::current().red,
                    )
                }
            }
            Some(ProviderAction::Switch) => {
                if self.app == AppType::Codex {
                    (
                        lang::current().confirm_switch_provider,
                        lang::current().confirm_switch_provider_msg,
                        theme::current().cyan,
                    )
                } else {
                    (
                        lang::current().confirm_switch_profile,
                        lang::current().confirm_switch_profile_msg,
                        theme::current().cyan,
                    )
                }
            }
            _ => return,
        };
        shared_confirm(
            f,
            area,
            title,
            msg,
            (
                lang::current().confirm_confirm,
                lang::current().confirm_cancel,
            ),
            (c, self.confirm_button),
        );
    }

    fn render_message_popup(&self, f: &mut Frame, area: Rect) {
        shared_msg(f, area, self.message.as_deref().unwrap_or(""));
    }

    fn render_content_popup(&mut self, f: &mut Frame, area: Rect) {
        let Some(popup_state) = self.content_popup.as_mut() else {
            return;
        };
        let content_width = popup_state
            .content
            .lines()
            .map(super::super::widgets::shared::display_width)
            .max()
            .unwrap_or(1);
        let width = if popup_state.compact {
            (content_width as u16)
                .saturating_add(6)
                .clamp(32, 72)
                .min(area.width)
        } else {
            area.width.saturating_sub(4).clamp(32, 100).min(area.width)
        };
        let available_content_width = width.saturating_sub(6).max(1) as usize;
        let wrapped_line_count = wrap_display_lines(&popup_state.content, available_content_width)
            .len()
            .min(u16::MAX as usize) as u16;
        let height = if popup_state.compact {
            wrapped_line_count
                .saturating_add(3)
                .clamp(6, 14)
                .min(area.height)
        } else {
            area.height.saturating_sub(4).clamp(9, 30).min(area.height)
        };
        let popup = super::super::widgets::shared::centered_rect(width, height, area);
        let block = Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(Line::from(format!(" {} ", popup_state.title)).centered())
            .border_style(Style::default().fg(theme::current().yellow));
        let inner = block.inner(popup);
        let [content_row, help_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .areas(inner);
        let [_, content_area, _] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(2.min(content_row.width / 4)),
                Constraint::Min(1),
                Constraint::Length(2.min(content_row.width / 4)),
            ])
            .areas(content_row);
        let lines = wrap_display_lines(&popup_state.content, content_area.width.max(1) as usize);
        popup_state.page_height = content_area.height.max(1);
        popup_state.max_scroll = lines
            .len()
            .saturating_sub(content_area.height as usize)
            .min(u16::MAX as usize) as u16;
        popup_state.scroll = popup_state.scroll.min(popup_state.max_scroll);

        clear_popup_area(f, popup);
        f.render_widget(block, popup);
        f.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .scroll((popup_state.scroll, 0)),
            content_area,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                if popup_state.compact {
                    lang::pick("Esc close", "Esc 关闭")
                } else {
                    lang::pick(
                        "J/K or Up/Down scroll · PgUp/PgDn page · Esc close",
                        "J/K 或上下键滚动 · PgUp/PgDn 翻页 · Esc 关闭",
                    )
                },
                Style::default().fg(theme::current().cyan),
            )))
            .alignment(Alignment::Center),
            help_area,
        );
    }

    fn render_selection_detail(&self, f: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        if let Some(provider) = self.selected_provider() {
            lines.push(detail_line(
                "Provider",
                &provider.name,
                theme::current().cyan,
            ));
            lines.push(detail_line("ID", &provider.id, theme::current().fg));
            lines.push(detail_line(
                "Base URL",
                &provider.api_url,
                theme::current().yellow,
            ));
            let key = if provider.api_key.starts_with("env:") {
                provider.api_key.clone()
            } else if provider.api_key.is_empty() {
                "-".into()
            } else {
                "••••••••".into()
            };
            lines.push(detail_line("API key", &key, theme::current().comment));
            if self.app == AppType::Codex {
                lines.push(Line::from(""));
                lines.push(detail_line(
                    "Wire API",
                    "responses",
                    theme::current().purple,
                ));
                lines.push(detail_line(
                    "Catalog",
                    provider.codex_catalog.as_str(),
                    theme::current().purple,
                ));
                lines.push(detail_line(
                    "Models",
                    &provider.models.len().to_string(),
                    theme::current().fg,
                ));
                if let Some(model) = self.selected_codex_model() {
                    lines.push(Line::from(""));
                    lines.push(detail_line(
                        "Model",
                        &model.display_name,
                        theme::current().cyan,
                    ));
                    lines.push(detail_line("Slug", &model.slug, theme::current().fg));
                    lines.push(detail_line(
                        "Context",
                        &format_context(model.context_window),
                        theme::current().yellow,
                    ));
                    lines.push(detail_line(
                        "Reasoning",
                        &model.supported_reasoning_efforts.join("/"),
                        theme::current().purple,
                    ));
                    lines.push(detail_line(
                        "Default",
                        &model.default_reasoning_effort,
                        theme::current().fg,
                    ));
                    lines.push(detail_line(
                        "Modalities",
                        &model.input_modalities.join(", "),
                        theme::current().fg,
                    ));
                }
            } else if let Some(profile) = self.selected_profile() {
                lines.push(Line::from(""));
                lines.push(detail_line("Profile", &profile.name, theme::current().cyan));
                lines.push(detail_line("Opus", &profile.opus, theme::current().fg));
                lines.push(detail_line("Sonnet", &profile.sonnet, theme::current().fg));
                lines.push(detail_line("Haiku", &profile.haiku, theme::current().fg));
                lines.push(detail_line(
                    "Subagent",
                    &profile.subagent,
                    theme::current().fg,
                ));
            }
        } else {
            lines.push(
                Line::from(Span::styled(
                    "No provider selected",
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
            );
        }
        let detail = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .title(lang::pick("Configuration Detail", "配置详情"))
                .border_style(Style::default().fg(theme::current().dim)),
        );
        f.render_widget(detail, area);
    }

    fn render_codex_model_list(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.panel == Panel::ProfileList;
        let block = Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(format!(
                "{} ({})",
                lang::current().models_title,
                self.codex_models.len()
            ))
            .border_style(Style::default().fg(if focused {
                theme::current().cyan
            } else {
                theme::current().dim
            }));
        if self
            .selected_provider()
            .is_some_and(|provider| provider.codex_catalog == CodexCatalog::BuiltIn)
        {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        lang::pick(
                            "Uses the Codex built-in model catalog",
                            "使用 Codex 内置模型目录",
                        ),
                        Style::default().fg(theme::current().comment),
                    ))
                    .centered(),
                ])
                .block(block),
                area,
            );
            return;
        }
        if self.codex_models.is_empty() {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        lang::pick("No third-party models configured", "尚未配置第三方模型"),
                        Style::default().fg(theme::current().comment),
                    ))
                    .centered(),
                ])
                .block(block),
                area,
            );
            return;
        }
        let items = self
            .codex_models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let selected = index == self.selected_profile_idx;
                let active = self.active_provider
                    == self
                        .selected_provider()
                        .map(|p| p.id.as_str())
                        .unwrap_or("")
                    && self.active_codex_model == model.slug;
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(
                                "{}{}",
                                if selected { "› " } else { "  " },
                                model.display_name
                            ),
                            Style::default().fg(if selected {
                                theme::current().cyan
                            } else {
                                theme::current().fg
                            }),
                        ),
                        if active {
                            Span::styled(" ●", Style::default().fg(theme::current().green))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(vec![
                        Span::raw("    "),
                        Span::styled(&model.slug, Style::default().fg(theme::current().comment)),
                        Span::styled(" · ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            format_context(model.context_window),
                            Style::default().fg(theme::current().comment),
                        ),
                        Span::styled(" · ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            &model.default_reasoning_effort,
                            Style::default().fg(theme::current().comment),
                        ),
                    ]),
                    Line::from(""),
                ])
            })
            .collect::<Vec<_>>();
        f.render_stateful_widget(
            List::new(items)
                .block(block)
                .highlight_style(Style::default()),
            area,
            &mut self.profile_state,
        );
    }
}

fn detail_line(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", format!("{}:", label)),
            Style::default().fg(theme::current().purple),
        ),
        Span::styled(value.to_string(), Style::default().fg(color)),
    ])
}

fn format_context(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 && tokens % 1_000 == 0 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

impl TabContent for ProvidersTab {
    fn render(&mut self, f: &mut Frame, area: Rect) {
        let popup_area = f.area();
        let (provider_area, profile_area, detail_area) = if self.app == AppType::Codex {
            if area.width >= 112 {
                let [providers, models, detail] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(30),
                        Constraint::Percentage(34),
                        Constraint::Percentage(36),
                    ])
                    .areas(area);
                (providers, models, detail)
            } else if area.width >= 86 {
                let [providers, right] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(area);
                let [models, detail] = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Min(10)])
                    .areas(right);
                (providers, models, detail)
            } else {
                let [providers, models, detail] = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(32),
                        Constraint::Percentage(32),
                        Constraint::Min(10),
                    ])
                    .areas(area);
                (providers, models, detail)
            }
        } else if area.width >= 96 {
            let [providers, right] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(area);
            let [profiles, detail] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Min(9)])
                .areas(right);
            (providers, profiles, detail)
        } else {
            let [providers, profiles, detail] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(34),
                    Constraint::Percentage(33),
                    Constraint::Min(9),
                ])
                .areas(area);
            (providers, profiles, detail)
        };

        // ── Left: Provider list ──
        let provider_items: Vec<ListItem> = self
            .providers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let is_sel = self.selected_provider_idx == i;
                let arrow = if is_sel { "› " } else { "  " };
                let tc = if is_sel {
                    theme::current().cyan
                } else {
                    theme::current().fg
                };
                let active = self.active_provider == p.id;
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, p.name), Style::default().fg(tc)),
                        if active {
                            Span::styled(" ●", Style::default().fg(theme::current().green))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(&p.id, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            source_label(p.source),
                            Style::default().fg(theme::current().comment),
                        ),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            if self.app == AppType::Codex {
                                if p.codex_catalog == CodexCatalog::Custom {
                                    format!("{} models", p.models.len())
                                } else {
                                    "built-in catalog".into()
                                }
                            } else {
                                format!("{} {}", p.profiles.len(), lang::current().profiles_count)
                            },
                            Style::default().fg(theme::current().comment),
                        ),
                    ]),
                    Line::from(""),
                ])
            })
            .collect();

        let prov_list = List::new(provider_items)
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!(
                        "{} ({})",
                        lang::current().providers_title,
                        self.providers.len()
                    ))
                    .border_style(if self.panel == Panel::ProviderList {
                        Style::default().fg(theme::current().cyan)
                    } else {
                        Style::default().fg(theme::current().dim)
                    }),
            )
            .highlight_style(Style::default());
        f.render_stateful_widget(prov_list, provider_area, &mut self.provider_state);

        if self.app == AppType::Codex {
            self.render_codex_model_list(f, profile_area);
            self.render_selection_detail(f, detail_area);
            if self.provider_form.is_some() {
                self.render_provider_form(f, popup_area);
            }
            if self.confirm_action.is_some() {
                self.render_confirm_popup(f, popup_area);
            }
            if self.codex_model_form.is_some() {
                self.render_codex_model_form(f, popup_area);
            }
            if self.message.is_some() {
                self.render_message_popup(f, popup_area);
            }
            if self.content_popup.is_some() {
                self.render_content_popup(f, popup_area);
            }
            return;
        }

        // ── Right: Profile list ──
        let profile_items: Vec<ListItem> = self
            .profiles
            .iter()
            .enumerate()
            .map(|(i, pr)| {
                let is_sel = self.selected_profile_idx == i;
                let arrow = if is_sel { "› " } else { "  " };
                let tc = if is_sel {
                    theme::current().cyan
                } else {
                    theme::current().fg
                };
                let active = self.selected_provider().map(|p| p.id.as_str())
                    == Some(self.active_provider.as_str())
                    && self.active_profile == pr.id;
                let opus = compact_model_name(&pr.opus);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{}{}", arrow, pr.name), Style::default().fg(tc)),
                        if active {
                            Span::styled(" ●", Style::default().fg(theme::current().green))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(vec![
                        Span::styled("     ", Style::default()),
                        Span::styled(&pr.id, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(opus, Style::default().fg(theme::current().comment)),
                        Span::styled(" \u{b7} ", Style::default().fg(theme::current().dim)),
                        Span::styled(
                            source_label(pr.source),
                            Style::default().fg(theme::current().comment),
                        ),
                    ]),
                    Line::from(""),
                ])
            })
            .collect();

        if self.profiles.is_empty() {
            let p = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    lang::current().no_profiles,
                    Style::default().fg(theme::current().comment),
                ))
                .centered(),
            ])
            .block(
                Block::bordered()
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .title(format!("{} (0)", lang::current().profiles_title))
                    .border_style(if self.panel == Panel::ProfileList {
                        Style::default().fg(theme::current().cyan)
                    } else {
                        Style::default().fg(theme::current().dim)
                    }),
            );
            f.render_widget(p, profile_area);
        } else {
            let prof_list = List::new(profile_items)
                .block(
                    Block::bordered()
                        .border_set(ratatui::symbols::border::ROUNDED)
                        .title(format!(
                            "{} ({})",
                            lang::current().profiles_title,
                            self.profiles.len()
                        ))
                        .border_style(if self.panel == Panel::ProfileList {
                            Style::default().fg(theme::current().cyan)
                        } else {
                            Style::default().fg(theme::current().dim)
                        }),
                )
                .highlight_style(Style::default());
            f.render_stateful_widget(prof_list, profile_area, &mut self.profile_state);
        }

        self.render_selection_detail(f, detail_area);

        // Popups
        if self.edit_form.is_some() {
            self.render_edit_form(f, popup_area);
        }
        if self.provider_form.is_some() {
            self.render_provider_form(f, popup_area);
        }
        if self.confirm_action.is_some() {
            self.render_confirm_popup(f, popup_area);
        }
        if self.message.is_some() {
            self.render_message_popup(f, popup_area);
        }
        if self.content_popup.is_some() {
            self.render_content_popup(f, popup_area);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        if let Some(popup) = self.content_popup.as_mut() {
            match code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.content_popup = None,
                KeyCode::Char('j') | KeyCode::Down => {
                    popup.scroll = popup.scroll.saturating_add(1).min(popup.max_scroll)
                }
                KeyCode::Char('k') | KeyCode::Up => popup.scroll = popup.scroll.saturating_sub(1),
                KeyCode::PageDown => {
                    popup.scroll = popup
                        .scroll
                        .saturating_add(popup.page_height)
                        .min(popup.max_scroll)
                }
                KeyCode::PageUp => popup.scroll = popup.scroll.saturating_sub(popup.page_height),
                KeyCode::Home => popup.scroll = 0,
                KeyCode::End => popup.scroll = popup.max_scroll,
                _ => {}
            }
            return true;
        }
        if self.message.is_some() {
            if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                self.message = None;
            }
            return true;
        }
        // Provider form mode
        if let Some(ref mut f) = self.provider_form {
            match code {
                KeyCode::Esc => {
                    self.provider_form = None;
                }
                KeyCode::Enter => {
                    self.commit_provider();
                }
                _ => {
                    f.handle_key(code);
                }
            }
            return true;
        }
        if let Some(ref mut form) = self.codex_model_form {
            match code {
                KeyCode::Esc => self.codex_model_form = None,
                KeyCode::Enter => self.commit_codex_model(),
                _ => form.handle_key(code),
            }
            return true;
        }
        // Edit form mode
        if let Some(ref mut f) = self.edit_form {
            match code {
                KeyCode::Esc => {
                    self.edit_form = None;
                }
                KeyCode::Enter => {
                    self.commit_edit();
                }
                _ => {
                    f.handle_key(code);
                }
            }
            return true;
        }
        if self.confirm_action.is_some() {
            match code {
                KeyCode::Tab | KeyCode::Right => {
                    self.confirm_button = (self.confirm_button + 1) % 2
                }
                KeyCode::BackTab | KeyCode::Left => {
                    self.confirm_button = if self.confirm_button == 0 { 1 } else { 0 }
                }
                KeyCode::Enter => {
                    if self.confirm_button == 0 {
                        match self.confirm_action {
                            Some(ProviderAction::Switch) => self.do_switch(),
                            Some(ProviderAction::Delete) => {
                                if self.panel == Panel::ProviderList {
                                    self.do_delete_provider();
                                } else if self.app == AppType::Codex {
                                    self.do_delete_codex_model();
                                } else {
                                    self.do_delete();
                                }
                            }
                            None => {}
                        }
                    }
                    self.confirm_action = None;
                    self.confirm_button = 0;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.confirm_action = None;
                    self.confirm_button = 0;
                }
                _ => {}
            }
            return true;
        }

        match self.panel {
            Panel::ProviderList => self.handle_provider_keys(code),
            Panel::ProfileList => self.handle_profile_keys(code),
        }
    }

    fn shortcut_groups(&self) -> Vec<Vec<(String, Color)>> {
        if self.app == AppType::Codex {
            return match self.panel {
                Panel::ProviderList => vec![
                    vec![
                        (" J/K ".into(), theme::current().comment),
                        (lang::current().sc_nav.into(), theme::current().comment),
                    ],
                    vec![
                        (" ⏎  ".into(), theme::current().comment),
                        (
                            lang::current().models_title.into(),
                            theme::current().comment,
                        ),
                    ],
                    vec![
                        (" A ".into(), theme::current().comment),
                        (lang::current().sc_add.into(), theme::current().comment),
                    ],
                    vec![
                        (" E ".into(), theme::current().comment),
                        (lang::current().sc_edit.into(), theme::current().comment),
                    ],
                    vec![
                        (" D ".into(), theme::current().comment),
                        (lang::current().sc_delete.into(), theme::current().comment),
                    ],
                    vec![
                        (" Q ".into(), theme::current().comment),
                        (lang::current().sc_quit.into(), theme::current().comment),
                    ],
                ],
                Panel::ProfileList => vec![
                    vec![
                        (" J/K ".into(), theme::current().comment),
                        (lang::current().sc_nav.into(), theme::current().comment),
                    ],
                    vec![
                        (" H/← ".into(), theme::current().comment),
                        (lang::current().sc_back.into(), theme::current().comment),
                    ],
                    vec![
                        (" ⏎  ".into(), theme::current().comment),
                        (lang::current().sc_switch.into(), theme::current().comment),
                    ],
                    vec![
                        (" A ".into(), theme::current().comment),
                        (lang::current().sc_add.into(), theme::current().comment),
                    ],
                    vec![
                        (" E ".into(), theme::current().comment),
                        (lang::current().sc_edit.into(), theme::current().comment),
                    ],
                    vec![
                        (" D ".into(), theme::current().comment),
                        (lang::current().sc_delete.into(), theme::current().comment),
                    ],
                    vec![
                        (" C ".into(), theme::current().comment),
                        (
                            lang::pick("Catalog", "目录").into(),
                            theme::current().comment,
                        ),
                    ],
                    vec![
                        (" V ".into(), theme::current().comment),
                        (
                            lang::pick("Preview", "预览").into(),
                            theme::current().comment,
                        ),
                    ],
                ],
            };
        }
        match self.panel {
            Panel::ProviderList => vec![
                vec![
                    (" J/K ".into(), theme::current().comment),
                    (lang::current().sc_nav.into(), theme::current().comment),
                ],
                vec![
                    (" L/→ ".into(), theme::current().comment),
                    (lang::current().sc_profiles.into(), theme::current().comment),
                ],
                vec![
                    (" A ".into(), theme::current().comment),
                    (lang::current().sc_add.into(), theme::current().comment),
                ],
                vec![
                    (" E ".into(), theme::current().comment),
                    (lang::current().sc_edit.into(), theme::current().comment),
                ],
                vec![
                    (" D ".into(), theme::current().comment),
                    (lang::current().sc_delete.into(), theme::current().comment),
                ],
                vec![
                    (" Q ".into(), theme::current().comment),
                    (lang::current().sc_quit.into(), theme::current().comment),
                ],
            ],
            Panel::ProfileList => vec![
                vec![
                    (" J/K ".into(), theme::current().comment),
                    (lang::current().sc_nav.into(), theme::current().comment),
                ],
                vec![
                    (" H/← ".into(), theme::current().comment),
                    (lang::current().sc_back.into(), theme::current().comment),
                ],
                vec![
                    (" ⏎  ".into(), theme::current().comment),
                    (lang::current().sc_switch.into(), theme::current().comment),
                ],
                vec![
                    (" A ".into(), theme::current().comment),
                    (lang::current().sc_add.into(), theme::current().comment),
                ],
                vec![
                    (" D ".into(), theme::current().comment),
                    (lang::current().sc_delete.into(), theme::current().comment),
                ],
                vec![
                    (" E ".into(), theme::current().comment),
                    (lang::current().sc_edit.into(), theme::current().comment),
                ],
                vec![
                    (" Q ".into(), theme::current().comment),
                    (lang::current().sc_quit.into(), theme::current().comment),
                ],
            ],
        }
    }
}

// ── Key handlers ──

impl ProvidersTab {
    fn handle_provider_keys(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('j') | KeyCode::Down => {
                let l = self.providers.len();
                if l > 0 {
                    self.selected_provider_idx = if self.selected_provider_idx + 1 < l {
                        self.selected_provider_idx + 1
                    } else {
                        0
                    };
                    self.provider_state.select(Some(self.selected_provider_idx));
                    self.load_profiles();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let l = self.providers.len();
                if l > 0 {
                    self.selected_provider_idx = if self.selected_provider_idx > 0 {
                        self.selected_provider_idx - 1
                    } else {
                        l - 1
                    };
                    self.provider_state.select(Some(self.selected_provider_idx));
                    self.load_profiles();
                }
            }
            KeyCode::Enter => {
                if self.app == AppType::Codex {
                    self.panel = Panel::ProfileList;
                    self.profile_state.select(if self.codex_models.is_empty() {
                        None
                    } else {
                        Some(self.selected_profile_idx)
                    });
                } else {
                    self.panel = Panel::ProfileList;
                    self.refresh_providers();
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.panel = Panel::ProfileList;
                let empty = if self.app == AppType::Codex {
                    self.codex_models.is_empty()
                } else {
                    self.profiles.is_empty()
                };
                self.profile_state.select(if empty {
                    None
                } else {
                    Some(self.selected_profile_idx)
                });
            }
            KeyCode::Char('a') | KeyCode::Char('A') => self.do_add_provider(),
            KeyCode::Char('e') | KeyCode::Char('E') => self.do_edit_provider(),
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some(prov) = self.selected_provider() {
                    if !prov.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_delete_sys_provider.into());
                    } else {
                        self.confirm_action = Some(ProviderAction::Delete);
                        self.confirm_button = 0;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn handle_profile_keys(&mut self, code: KeyCode) -> bool {
        if self.app == AppType::Codex {
            return self.handle_codex_model_keys(code);
        }
        if self.is_searching {
            match code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => self.is_searching = false,
                KeyCode::Backspace | KeyCode::Delete => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.panel = Panel::ProviderList;
                self.provider_state.select(Some(self.selected_provider_idx));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let l = self.profiles.len();
                if l > 0 {
                    self.selected_profile_idx = if self.selected_profile_idx + 1 < l {
                        self.selected_profile_idx + 1
                    } else {
                        0
                    };
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let l = self.profiles.len();
                if l > 0 {
                    self.selected_profile_idx = if self.selected_profile_idx > 0 {
                        self.selected_profile_idx - 1
                    } else {
                        l - 1
                    };
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Enter => {
                if !self.profiles.is_empty() {
                    self.confirm_action = Some(ProviderAction::Switch);
                    self.confirm_button = 0;
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if self.profiles.is_empty() {
                    return false;
                }
                if let Some(pr) = self.selected_profile() {
                    if !pr.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_delete_sys_profile.into());
                        return true;
                    }
                }
                self.confirm_action = Some(ProviderAction::Delete);
                self.confirm_button = 0;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                if self.profiles.is_empty() {
                    return false;
                }
                if let Some(pr) = self.selected_profile() {
                    if !pr.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_edit_sys_profile.into());
                        return true;
                    }
                }
                self.do_edit();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => self.do_add_profile(),
            _ => return false,
        }
        true
    }

    fn handle_codex_model_keys(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.panel = Panel::ProviderList;
                self.provider_state.select(Some(self.selected_provider_idx));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.codex_models.len();
                if len > 0 {
                    self.selected_profile_idx = (self.selected_profile_idx + 1) % len;
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let len = self.codex_models.len();
                if len > 0 {
                    self.selected_profile_idx = if self.selected_profile_idx == 0 {
                        len - 1
                    } else {
                        self.selected_profile_idx - 1
                    };
                    self.profile_state.select(Some(self.selected_profile_idx));
                }
            }
            KeyCode::Enter => {
                let built_in = self
                    .selected_provider()
                    .is_some_and(|provider| provider.codex_catalog == CodexCatalog::BuiltIn);
                if built_in || !self.codex_models.is_empty() {
                    self.confirm_action = Some(ProviderAction::Switch);
                    self.confirm_button = 0;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => self.do_add_codex_model(),
            KeyCode::Char('e') | KeyCode::Char('E') => self.do_edit_codex_model(),
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some(model) = self.selected_codex_model() {
                    if !model.source.can_delete() {
                        self.message = Some(lang::current().msg_cannot_delete_sys_profile.into());
                    } else {
                        self.confirm_action = Some(ProviderAction::Delete);
                        self.confirm_button = 0;
                    }
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => self.show_catalog_status(),
            KeyCode::Char('v') | KeyCode::Char('V') => self.preview_codex_model(),
            _ => return false,
        }
        true
    }
}

fn wrap_display_lines(content: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut output = Vec::new();
    for source in content.lines() {
        if source.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut line_width = 0usize;
        for ch in source.chars() {
            let char_width = super::super::widgets::shared::display_width(&ch.to_string());
            if line_width + char_width > width && !line.is_empty() {
                output.push(std::mem::take(&mut line));
                line_width = 0;
            }
            line.push(ch);
            line_width += char_width;
        }
        output.push(line);
    }
    if output.is_empty() {
        output.push(String::new());
    }
    output
}

fn source_label(s: crate::core::models::Source) -> &'static str {
    if s.can_delete() {
        lang::current().label_user
    } else {
        lang::current().label_system
    }
}

fn localized_error(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<crate::core::env::ApiKeyUnavailable>() {
        return lang::pick_owned(
            format!("Error: {}", error),
            format!(
                "错误：供应商 '{}' 的 API 密钥不可用。请设置 {}，或使用明文密钥。",
                error.provider_id, error.env_var
            ),
        );
    }
    lang::pick_owned(format!("Error: {}", error), format!("错误：{}", error))
}

fn compact_model_name(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        "-".into()
    } else {
        model
            .replace("[1m]", " 1M")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::compact_model_name;

    #[test]
    fn formats_profile_opus_model_for_list() {
        assert_eq!(
            compact_model_name("deepseek-v4-pro[1m]"),
            "deepseek-v4-pro 1M"
        );
        assert_eq!(compact_model_name(" claude-opus-4 "), "claude-opus-4");
        assert_eq!(compact_model_name(""), "-");
    }
}
