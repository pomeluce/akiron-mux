use std::sync::atomic::{AtomicU8, Ordering};

pub struct Lang {
    // Sidebar
    pub tab_providers: &'static str,
    pub tab_usage: &'static str,
    pub tab_history: &'static str,
    pub tab_settings: &'static str,

    // Shortcut bar
    pub sc_nav: &'static str,
    pub sc_quit: &'static str,
    pub sc_search: &'static str,
    pub sc_switch: &'static str,
    pub sc_delete: &'static str,
    pub sc_edit: &'static str,
    pub sc_add: &'static str,
    pub sc_back: &'static str,
    pub sc_open: &'static str,
    pub sc_toggle: &'static str,
    pub sc_scroll: &'static str,
    pub sc_profiles: &'static str,
    pub sc_save: &'static str,
    pub sc_cancel: &'static str,
    pub sc_next_field: &'static str,

    // Provider tab
    pub providers_title: &'static str,
    pub profiles_title: &'static str,
    pub no_profiles: &'static str,
    pub label_system: &'static str,
    pub label_user: &'static str,
    pub label_profile_id: &'static str,
    pub label_profile_name: &'static str,
    pub label_opus: &'static str,
    pub label_sonnet: &'static str,
    pub label_haiku: &'static str,
    pub label_subagent: &'static str,
    pub label_prov_name: &'static str,
    pub label_prov_id: &'static str,
    pub label_api_url: &'static str,
    pub label_api_key: &'static str,
    pub title_edit_profile: &'static str,
    pub title_add_profile: &'static str,
    pub title_edit_provider: &'static str,
    pub title_add_provider: &'static str,
    pub confirm_delete_profile: &'static str,
    pub confirm_delete_provider: &'static str,
    pub confirm_delete_profile_msg: &'static str,
    pub confirm_delete_provider_msg: &'static str,
    pub confirm_switch_profile: &'static str,
    pub confirm_switch_provider: &'static str,
    pub confirm_switch_profile_msg: &'static str,
    pub confirm_switch_provider_msg: &'static str,
    pub confirm_confirm: &'static str,
    pub confirm_cancel: &'static str,
    pub confirm_ok: &'static str,
    pub confirm_delete_title: &'static str,
    pub confirm_delete_session_msg: &'static str,
    pub confirm_open_title: &'static str,
    pub confirm_open_claude_msg: &'static str,
    pub confirm_open_codex_msg: &'static str,
    pub notice_title: &'static str,
    pub msg_cannot_delete_sys_profile: &'static str,
    pub msg_cannot_edit_sys_profile: &'static str,
    pub msg_cannot_delete_sys_provider: &'static str,
    pub msg_cannot_edit_sys_provider: &'static str,

    // Usage tab
    pub chart_title: &'static str,
    pub models_title: &'static str,
    pub usage_tab_title: &'static str,
    pub card_today: &'static str,
    pub card_week: &'static str,
    pub card_total: &'static str,
    pub card_reqs: &'static str,
    pub no_usage: &'static str,
    pub no_usage_hint: &'static str,
    pub no_usage_7d: &'static str,
    pub chart_input: &'static str,
    pub chart_output: &'static str,
    pub chart_cache_read: &'static str,
    pub chart_cache_create: &'static str,

    // History tab
    pub sessions_title: &'static str,
    pub detail_title: &'static str,
    pub detail_project: &'static str,
    pub detail_provider: &'static str,
    pub detail_profile: &'static str,
    pub detail_mode: &'static str,
    pub detail_tokens: &'static str,
    pub detail_started: &'static str,
    pub detail_messages: &'static str,
    pub detail_size: &'static str,
    pub no_sessions: &'static str,
    pub search_hint: &'static str,

    // Settings tab
    pub settings_title: &'static str,
    pub setting_theme: &'static str,
    pub setting_mode: &'static str,
    pub setting_language: &'static str,
    pub setting_session_service: &'static str,

    // App bar

    // Confirm popup
    // Profile count
    pub profiles_count: &'static str,
}

pub const LANGS: &[(&str, &Lang)] = &[("English", &LANG_EN), ("中文", &LANG_ZH)];

const LANG_ZH: Lang = Lang {
    tab_providers: "模型",
    tab_usage: "用量",
    tab_history: "会话",
    tab_settings: "设置",

    sc_nav: "导航",
    sc_quit: "退出",
    sc_search: "搜索",
    sc_switch: "切换",
    sc_delete: "删除",
    sc_edit: "编辑",
    sc_add: "添加",
    sc_back: "返回",
    sc_open: "打开",
    sc_toggle: "切换",
    sc_scroll: "滚动",
    sc_profiles: "配置",
    sc_save: "保存",
    sc_cancel: "取消",
    sc_next_field: "下一项",

    providers_title: "供应商",
    profiles_title: "模型配置",
    no_profiles: "没有可用配置",
    label_system: "系统",
    label_user: "用户",
    label_profile_id: "配置 ID",
    label_profile_name: "配置名称",
    label_opus: "Opus 模型",
    label_sonnet: "Sonnet 模型",
    label_haiku: "Haiku 模型",
    label_subagent: "子代理模型",
    label_prov_name: "名称",
    label_prov_id: "ID",
    label_api_url: "API 地址",
    label_api_key: "API 密钥",
    title_edit_profile: " 编辑配置 ",
    title_add_profile: " 添加配置 ",
    title_edit_provider: " 编辑供应商 ",
    title_add_provider: " 添加供应商 ",
    confirm_delete_profile: " 删除配置 ",
    confirm_delete_provider: " 删除供应商 ",
    confirm_delete_profile_msg: " 删除此配置？ ",
    confirm_delete_provider_msg: " 删除此供应商？ ",
    confirm_switch_profile: " 切换配置 ",
    confirm_switch_provider: " 切换供应商 ",
    confirm_switch_profile_msg: " 切换到此模型配置？ ",
    confirm_switch_provider_msg: " 切换到此供应商？ ",
    confirm_confirm: "确认",
    notice_title: " 提示 ",
    confirm_cancel: "取消",
    confirm_ok: "确定",
    confirm_delete_title: " 删除会话 ",
    confirm_delete_session_msg: " 删除此会话？ ",
    confirm_open_title: " 打开会话 ",
    confirm_open_claude_msg: " 在 Claude Code 中打开此会话？ ",
    confirm_open_codex_msg: " 在 Codex 中打开此会话？ ",
    msg_cannot_delete_sys_profile: "无法删除系统默认配置",
    msg_cannot_edit_sys_profile: "无法编辑系统默认配置",
    msg_cannot_delete_sys_provider: "无法删除系统默认供应商",
    msg_cannot_edit_sys_provider: "无法编辑系统默认供应商",

    chart_title: "本周",
    models_title: "模型",
    usage_tab_title: "用量",
    card_today: "今日",
    card_week: "本周",
    card_total: "总计",
    card_reqs: "请求数",
    no_usage: "暂无用量数据",
    no_usage_hint: "首次启动后自动开始扫描",
    no_usage_7d: "最近 7 天没有使用该模型",
    chart_input: "输入",
    chart_output: "输出",
    chart_cache_read: "缓存读取",
    chart_cache_create: "缓存创建",

    sessions_title: "会话",
    detail_title: "会话详情",
    detail_project: "项目",
    detail_provider: "供应商",
    detail_profile: "配置",
    detail_mode: "模式",
    detail_tokens: "Tokens",
    detail_started: "开始时间",
    detail_messages: "消息数",
    detail_size: "大小",
    no_sessions: "没有可用会话",
    search_hint: "搜索 (/ 聚焦)",

    settings_title: "设置",
    setting_theme: "主题",
    setting_mode: "模式",
    setting_language: "语言",
    setting_session_service: "会话后端",

    profiles_count: "个配置",
};

const LANG_EN: Lang = Lang {
    tab_providers: "Models",
    tab_usage: "Usage",
    tab_history: "History",
    tab_settings: "Settings",

    sc_nav: "Nav",
    sc_quit: "Quit",
    sc_search: "Search",
    sc_switch: "Switch",
    sc_delete: "Del",
    sc_edit: "Edit",
    sc_add: "Add",
    sc_back: "Back",
    sc_open: "Open",
    sc_toggle: "Toggle",
    sc_scroll: "Scroll",
    sc_profiles: "Profiles",
    sc_save: "Save",
    sc_cancel: "Cancel",
    sc_next_field: "Next field",

    providers_title: "Providers",
    profiles_title: "Profiles",
    no_profiles: "No profiles configured",
    label_system: "system",
    label_user: "user",
    label_profile_id: "Profile ID",
    label_profile_name: "Profile Name",
    label_opus: "Opus Model",
    label_sonnet: "Sonnet Model",
    label_haiku: "Haiku Model",
    label_subagent: "Subagent Model",
    label_prov_name: "Name",
    label_prov_id: "ID",
    label_api_url: "API URL",
    label_api_key: "API Key",
    title_edit_profile: " Edit Profile ",
    title_add_profile: " Add Profile ",
    title_edit_provider: " Edit Provider ",
    title_add_provider: " Add Provider ",
    confirm_delete_profile: " Delete Profile ",
    confirm_delete_provider: " Delete Provider ",
    confirm_delete_profile_msg: " Delete this profile? ",
    confirm_delete_provider_msg: " Delete this provider? ",
    confirm_switch_profile: " Switch Profile ",
    confirm_switch_provider: " Switch Provider ",
    confirm_switch_profile_msg: " Switch to this model profile? ",
    confirm_switch_provider_msg: " Switch to this provider? ",
    notice_title: " Notice ",
    confirm_confirm: "Confirm",
    confirm_cancel: "Cancel",
    confirm_ok: "OK",
    confirm_delete_title: " Confirm Delete ",
    confirm_delete_session_msg: " Delete this session? ",
    confirm_open_title: " Open Session ",
    confirm_open_claude_msg: " Open this session in Claude Code? ",
    confirm_open_codex_msg: " Open this session in Codex? ",
    msg_cannot_delete_sys_profile: "Cannot delete system default profile",
    msg_cannot_edit_sys_profile: "Cannot edit system default profile",
    msg_cannot_delete_sys_provider: "Cannot delete system default provider",
    msg_cannot_edit_sys_provider: "Cannot edit system default provider",

    chart_title: "This Week",
    models_title: "Models",
    usage_tab_title: " Usage ",
    card_today: "Today",
    card_week: "Week",
    card_total: "Total",
    card_reqs: "Reqs",
    no_usage: "No usage data yet",
    no_usage_hint: "Scan starts automatically on first launch",
    no_usage_7d: "No usage in last 7 days",
    chart_input: "input",
    chart_output: "output",
    chart_cache_read: "cache read",
    chart_cache_create: "cache create",

    sessions_title: "Sessions",
    detail_title: "Session Detail",
    detail_project: "Project",
    detail_provider: "Provider",
    detail_profile: "Profile",
    detail_mode: "Mode",
    detail_tokens: "Tokens",
    detail_started: "Started",
    detail_messages: "Messages",
    detail_size: "Size",
    no_sessions: "No sessions available",
    search_hint: "Search (/ to focus)",

    settings_title: "Settings",
    setting_theme: "Theme",
    setting_mode: "Mode",
    setting_language: "Language",
    setting_session_service: "Session backend",

    profiles_count: "profiles",
};

static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn current() -> &'static Lang {
    let idx = ACTIVE.load(Ordering::Relaxed) as usize;
    LANGS[idx.min(LANGS.len() - 1)].1
}

pub fn set_lang(name: &str) -> bool {
    if let Some(idx) = LANGS.iter().position(|(n, _)| *n == name) {
        ACTIVE.store(idx as u8, Ordering::Relaxed);
        true
    } else {
        false
    }
}

pub fn current_lang() -> &'static str {
    let idx = ACTIVE.load(Ordering::Relaxed) as usize;
    LANGS[idx.min(LANGS.len() - 1)].0
}

pub fn pick(en: &'static str, zh: &'static str) -> &'static str {
    if current_lang() == "中文" {
        zh
    } else {
        en
    }
}

pub fn pick_owned(en: String, zh: String) -> String {
    if current_lang() == "中文" {
        zh
    } else {
        en
    }
}
