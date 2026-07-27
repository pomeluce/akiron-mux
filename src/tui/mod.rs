pub mod app;
pub mod file_watcher;
pub mod lang;
pub mod tabs;
pub mod theme;
pub mod widgets;

use crate::core::config;

pub fn run_tui() -> anyhow::Result<()> {
    let db_path = config::db_path();
    let defaults_path = config::defaults_path();

    let mut app = app::App::new(&db_path, defaults_path.as_deref())?;
    app.run()
}
