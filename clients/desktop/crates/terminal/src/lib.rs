//! Terminal model and GPUI rendering surface for the installed Desktop Client.

mod input;
mod model;
mod view;

pub use input::{KeyModifiers, encode_key, encode_paste};
pub use model::{RenderCell, RenderSnapshot, TerminalDimensions, TerminalModel, TerminalSnapshot};
pub use view::TerminalView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalNotice {
    Attention(akmux_client_core::AttentionKind),
    Status(akmux_client_core::SessionInfo),
    AuthorizationRevoked,
    OpenLink(String),
}
