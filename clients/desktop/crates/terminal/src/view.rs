use std::ops::Range;

use akmux_client_core::{LeaseState, TerminalCommand, TerminalConnectionPhase, TerminalEvent, TerminalTransport, TerminalTransportConfig};
use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::{KeyModifiers, TerminalDimensions, TerminalModel, TerminalNotice, encode_key, encode_paste};

const DEFAULT_ROWS: usize = 36;
const DEFAULT_COLUMNS: usize = 120;

pub struct TerminalView {
    focus_handle: FocusHandle,
    model: TerminalModel,
    _transport: TerminalTransport,
    commands: async_channel::Sender<TerminalCommand>,
    phase: TerminalConnectionPhase,
    lease: Option<LeaseState>,
    can_write: bool,
    simplified_chinese: bool,
    font_size: f32,
    marked_text: String,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    _event_task: Task<()>,
}

impl TerminalView {
    pub fn new(config: TerminalTransportConfig, runtime: &tokio::runtime::Handle, cx: &mut Context<Self>) -> Self {
        let transport = TerminalTransport::spawn(config, runtime);
        let commands = transport.commands();
        let events = transport.events();
        let _event_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |this, cx| this.handle_event(event, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            model: TerminalModel::new(TerminalDimensions {
                rows: DEFAULT_ROWS,
                columns: DEFAULT_COLUMNS,
            }),
            _transport: transport,
            commands,
            phase: TerminalConnectionPhase::Connecting,
            lease: None,
            can_write: false,
            simplified_chinese: false,
            font_size: 12.0,
            marked_text: String::new(),
            last_bounds: None,
            selecting: false,
            _event_task,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn resize(&mut self, rows: usize, columns: usize, cx: &mut Context<Self>) {
        let dimensions = TerminalDimensions {
            rows: rows.max(1),
            columns: columns.max(2),
        };
        self.model.resize(dimensions);
        let _ = self.commands.try_send(TerminalCommand::Resize {
            rows: dimensions.rows.min(u16::MAX as usize) as u16,
            cols: dimensions.columns.min(u16::MAX as usize) as u16,
        });
        cx.notify();
    }

    pub fn take_control(&self) {
        if let Some(lease) = &self.lease {
            let _ = self.commands.try_send(TerminalCommand::TakeControl { expected_version: lease.version });
        }
    }

    pub fn set_font_size(&mut self, font_size: f32, cx: &mut Context<Self>) {
        self.font_size = font_size.clamp(9.0, 24.0);
        cx.notify();
    }

    pub fn set_simplified_chinese(&mut self, simplified_chinese: bool, cx: &mut Context<Self>) {
        self.simplified_chinese = simplified_chinese;
        cx.notify();
    }

    fn handle_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Output { bytes, replace } => {
                if replace {
                    self.model.replace(&bytes);
                } else {
                    self.model.feed(&bytes);
                }
                for response in self.model.take_pty_writes() {
                    let _ = self.commands.try_send(TerminalCommand::Input(response));
                }
            }
            TerminalEvent::Lease { lease, can_write } => {
                self.lease = Some(lease);
                self.can_write = can_write;
            }
            TerminalEvent::AuthorizationRevoked => {
                self.can_write = false;
                cx.emit(TerminalNotice::AuthorizationRevoked);
            }
            TerminalEvent::Phase(phase) => self.phase = phase,
            TerminalEvent::ProtocolError { message, .. } => {
                self.model.feed(format!("\r\n\x1b[31m{message}\x1b[0m\r\n").as_bytes());
            }
            TerminalEvent::Status(session) => cx.emit(TerminalNotice::Status(session)),
            TerminalEvent::Attention(kind) => cx.emit(TerminalNotice::Attention(kind)),
            TerminalEvent::RecoveryCredential(_) => {}
        }
        cx.notify();
    }

    fn send_text(&self, text: &str) {
        if !text.is_empty() {
            let _ = self.commands.try_send(TerminalCommand::Input(text.as_bytes().to_vec()));
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let modifiers = KeyModifiers {
            control: stroke.modifiers.control,
            alt: stroke.modifiers.alt,
            shift: stroke.modifiers.shift,
        };
        let secondary = stroke.modifiers.secondary();
        if secondary
            && stroke.key.eq_ignore_ascii_case("c")
            && let Some(text) = self.model.selected_text().filter(|text| !text.is_empty())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.stop_propagation();
            return;
        }
        if secondary && stroke.key.eq_ignore_ascii_case("v") {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let bytes = encode_paste(&text, self.model.bracketed_paste_mode());
                let _ = self.commands.try_send(TerminalCommand::Input(bytes));
            }
            cx.stop_propagation();
            return;
        }

        let special = matches!(
            stroke.key.as_str(),
            "enter"
                | "tab"
                | "backspace"
                | "escape"
                | "up"
                | "down"
                | "right"
                | "left"
                | "home"
                | "end"
                | "insert"
                | "delete"
                | "pageup"
                | "pagedown"
                | "f1"
                | "f2"
                | "f3"
                | "f4"
                | "f5"
                | "f6"
                | "f7"
                | "f8"
                | "f9"
                | "f10"
                | "f11"
                | "f12"
        );
        if (special || modifiers.control || modifiers.alt)
            && let Some(bytes) = encode_key(&stroke.key, stroke.key_char.as_deref(), modifiers, self.model.application_cursor_mode())
        {
            let _ = self.commands.try_send(TerminalCommand::Input(bytes));
            cx.stop_propagation();
        }
    }

    fn phase_label(&self) -> &'static str {
        match (self.phase, self.simplified_chinese) {
            (TerminalConnectionPhase::RequestingTicket, true) => "正在认证",
            (TerminalConnectionPhase::Connecting, true) => "正在连接",
            (TerminalConnectionPhase::Open, true) => {
                if self.can_write {
                    "已连接"
                } else {
                    "只读"
                }
            }
            (TerminalConnectionPhase::Reconnecting, true) => "正在重连",
            (TerminalConnectionPhase::Revoked, true) => "访问已撤销",
            (TerminalConnectionPhase::Disposed, true) => "已断开",
            (TerminalConnectionPhase::RequestingTicket, false) => "Authenticating",
            (TerminalConnectionPhase::Connecting, false) => "Connecting",
            (TerminalConnectionPhase::Open, false) => {
                if self.can_write {
                    "Connected"
                } else {
                    "Read only"
                }
            }
            (TerminalConnectionPhase::Reconnecting, false) => "Reconnecting",
            (TerminalConnectionPhase::Revoked, false) => "Access revoked",
            (TerminalConnectionPhase::Disposed, false) => "Disconnected",
        }
    }

    fn terminal_position(&self, position: Point<Pixels>) -> (usize, usize) {
        let bounds = self.last_bounds.unwrap_or_else(|| Bounds::new(point(px(0.), px(0.)), size(px(1.), px(1.))));
        let x = (f32::from(position.x - bounds.left()) / (self.font_size * 0.62)).floor().max(0.) as usize;
        let y = (f32::from(position.y - bounds.top()) / (self.font_size * 1.45)).floor().max(0.) as usize;
        (y, x)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let (row, column) = self.terminal_position(event.position);
        if self.model.mouse_reporting_mode() {
            self.send_mouse(mouse_button_code(event.button, event.modifiers), row, column, false);
        } else if event.button == MouseButton::Left {
            self.model.start_selection(row, column, event.click_count);
            self.selecting = true;
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !event.dragging() {
            return;
        }
        let (row, column) = self.terminal_position(event.position);
        if self.model.mouse_reporting_mode() {
            self.send_mouse(32 + mouse_button_code(MouseButton::Left, event.modifiers), row, column, false);
        } else if self.selecting {
            self.model.update_selection(row, column);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (row, column) = self.terminal_position(event.position);
        if self.model.mouse_reporting_mode() {
            self.send_mouse(mouse_button_code(event.button, event.modifiers), row, column, true);
        } else if event.button == MouseButton::Left {
            self.selecting = false;
            if let Some(text) = self.model.selected_text().filter(|text| !text.is_empty()) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            } else if let Some(link) = self.model.hyperlink_at(row, column) {
                cx.emit(TerminalNotice::OpenLink(link));
            }
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(self.font_size * 1.45));
        let lines = (f32::from(delta.y) / (self.font_size * 1.45)).round() as i32;
        if lines == 0 {
            return;
        }
        if self.model.mouse_reporting_mode() {
            let (row, column) = self.terminal_position(event.position);
            let button = if lines > 0 { 65 } else { 64 };
            for _ in 0..lines.unsigned_abs().min(8) {
                self.send_mouse(button, row, column, false);
            }
        } else {
            self.model.scroll(-lines);
            cx.notify();
        }
    }

    fn send_mouse(&self, button: u8, row: usize, column: usize, release: bool) {
        let sequence = if self.model.sgr_mouse_mode() {
            format!("\x1b[<{button};{};{}{}", column + 1, row + 1, if release { 'm' } else { 'M' }).into_bytes()
        } else {
            vec![
                0x1b,
                b'[',
                b'M',
                32_u8.saturating_add(if release { 3 } else { button }),
                32_u8.saturating_add((column + 1).min(223) as u8),
                32_u8.saturating_add((row + 1).min(223) as u8),
            ]
        };
        let _ = self.commands.try_send(TerminalCommand::Input(sequence));
    }
}

impl EventEmitter<TerminalNotice> for TerminalView {}

fn mouse_button_code(button: MouseButton, modifiers: Modifiers) -> u8 {
    let button = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Navigate(_) => 0,
    };
    button + modifiers.shift as u8 * 4 + modifiers.alt as u8 * 8 + modifiers.control as u8 * 16
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let utf16 = self.marked_text.encode_utf16().collect::<Vec<_>>();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        actual.replace(range.clone());
        Some(String::from_utf16_lossy(&utf16[range]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked_text.encode_utf16().count();
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked_text.is_empty()).then(|| 0..self.marked_text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text.clear();
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.send_text(text);
        self.marked_text.clear();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text.clear();
        self.marked_text.push_str(text);
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds.unwrap_or(bounds);
        Some(Bounds::new(point(bounds.left(), bounds.bottom() - px(20.)), size(px(2.), px(20.))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.marked_text.encode_utf16().count())
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.marked_text.encode_utf16().count())
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.model.render_snapshot();
        let font = window.text_style().font();
        let row_height = px(self.font_size * 1.45);
        let rows = (0..snapshot.rows).map(|line| {
            let cells = snapshot.cells.iter().filter(|cell| cell.line == line as i32);
            let mut text = String::with_capacity(snapshot.columns);
            let mut runs = Vec::with_capacity(snapshot.columns);
            for cell in cells {
                text.push_str(&cell.text);
                let background = if cell.cursor {
                    0xD8DEE9
                } else if cell.selected {
                    0x334155
                } else {
                    cell.background
                };
                let foreground = if cell.cursor { 0x0F1117 } else { cell.foreground };
                runs.push(TextRun {
                    len: cell.text.len(),
                    font: Font {
                        weight: if cell.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
                        style: if cell.italic { FontStyle::Italic } else { FontStyle::Normal },
                        ..font.clone()
                    },
                    color: rgb(foreground).into(),
                    background_color: Some(rgb(background).into()),
                    underline: cell.underline.then(|| UnderlineStyle {
                        color: Some(rgb(foreground).into()),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    strikethrough: None,
                });
            }
            div().h(row_height).line_height(row_height).child(StyledText::new(text).with_runs(runs))
        });

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0F1117))
            .text_size(px(self.font_size))
            .font_family("Maple Mono")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(div().absolute().top_0().left_0().right_0().bottom_0().child(TerminalInputElement { view: cx.entity() }))
            .child(div().absolute().top_2().right_3().text_xs().text_color(rgb(0x7F8C98)).child(self.phase_label()))
            .children(rows)
            .when(!self.marked_text.is_empty(), |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom_2()
                        .left_2()
                        .px_2()
                        .bg(rgb(0x263040))
                        .text_color(rgb(0xD8DEE9))
                        .child(self.marked_text.clone()),
                )
            })
    }
}

struct TerminalInputElement {
    view: Entity<TerminalView>,
}

impl IntoElement for TerminalInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalInputElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut Window, cx: &mut App) {
        self.view.update(cx, |view, cx| {
            view.last_bounds = Some(bounds);
            let columns = ((f32::from(bounds.size.width) / (view.font_size * 0.62)).floor() as usize).max(2);
            let rows = ((f32::from(bounds.size.height) / (view.font_size * 1.45)).floor() as usize).max(1);
            let snapshot = view.model.snapshot();
            if snapshot.rows != rows || snapshot.columns != columns {
                view.resize(rows, columns, cx);
            }
        });
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus = self.view.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
    }
}
