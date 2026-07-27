pub mod history;
pub mod providers;
pub mod settings;
pub mod usage;

use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Providers,
    Usage,
    History,
    Settings,
}

pub trait TabContent {
    fn render(&mut self, f: &mut Frame, area: ratatui::layout::Rect);
    fn handle_key(&mut self, code: ratatui::crossterm::event::KeyCode) -> bool;
    /// Shortcut key groups for the global shortcut bar: [(key, label_color), ...]
    fn shortcut_groups(&self) -> Vec<Vec<(String, ratatui::style::Color)>>;
}
