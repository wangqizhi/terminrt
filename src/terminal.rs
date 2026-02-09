use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, Color as TermColor, NamedColor};

use winit::keyboard::{Key, NamedKey};

use crate::pty::{self, PtySize, PtyWriter};

pub const TERM_FONT_SIZE: f32 = 14.0;
const VT_LOG_MAX_LINES: usize = 2000;
const MAX_SELECTION_COPY_BYTES: usize = 2 * 1024 * 1024;
const CWD_OSC_PREFIX: &[u8] = b"\x1b]633;CWD=";
const OSC_BEL: u8 = 0x07;
const OSC_ST: &[u8] = b"\x1b\\";

#[derive(Clone, Debug)]
pub enum VtLogEntry {
    Input(String),
    Output(String),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TerminalSelectionState {
    anchor: Option<(usize, usize)>,
    focus: Option<(usize, usize)>,
    dragging: bool,
}

impl TerminalSelectionState {
    pub fn clear(&mut self) {
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
    }

    fn start(&mut self, row: usize, col: usize) {
        self.anchor = Some((row, col));
        self.focus = Some((row, col));
        self.dragging = true;
    }

    fn update(&mut self, row: usize, col: usize) {
        if self.anchor.is_some() {
            self.focus = Some((row, col));
        }
    }

    fn stop_dragging(&mut self) {
        self.dragging = false;
    }

    fn normalized(&self) -> Option<((usize, usize), (usize, usize))> {
        let mut start = self.anchor?;
        let mut end = self.focus?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        Some((start, end))
    }

    pub fn has_selection(&self) -> bool {
        matches!(self.normalized(), Some((start, end)) if start != end)
    }

}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollRequest {
    /// Scroll so the top of the terminal screen (after scrollback) is visible.
    ScreenTop,
    /// Scroll so the current cursor line is aligned to the top.
    CursorTop,
    /// Scroll so the current cursor line is visible while typing.
    CursorLine,
}

#[derive(Copy, Clone)]
struct TermDims {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct TerminalInstance {
    term: Term<VoidListener>,
    processor: ansi::Processor,
    rx: mpsc::Receiver<Vec<u8>>,
    pty_writer: Arc<Mutex<PtyWriter>>,
    vt_lines: VecDeque<VtLogEntry>,
    vt_pending: String,
    osc_tracking_buffer: Vec<u8>,
    current_dir: String,
    _reader_thread: thread::JoinHandle<()>,
}

pub struct ProcessInputResult {
    pub had_input: bool,
    pub pty_closed: bool,
}

impl TerminalInstance {
    pub fn new(rows: u16, cols: u16, startup_dir: PathBuf) -> io::Result<Self> {
        let size = PtySize { rows, cols };
        let (mut reader, writer) = pty::spawn_pty(size, &startup_dir)?;
        let pty_writer = Arc::new(Mutex::new(writer));

        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        // Reader thread owns the PtyReader directly — no mutex needed
        let reader_thread = thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let config = Config::default();
        let dims = TermDims {
            cols: cols as usize,
            rows: rows as usize,
        };
        let term = Term::new(config, &dims, VoidListener);
        let processor = ansi::Processor::new();

        Ok(Self {
            term,
            processor,
            rx,
            pty_writer,
            vt_lines: VecDeque::new(),
            vt_pending: String::new(),
            osc_tracking_buffer: Vec::new(),
            current_dir: startup_dir.display().to_string(),
            _reader_thread: reader_thread,
        })
    }

    /// Process pending PTY output, feeding bytes into the terminal emulator.
    pub fn process_input(&mut self) -> ProcessInputResult {
        let mut had_input = false;
        let mut pty_closed = false;
        loop {
            match self.rx.try_recv() {
                Ok(data) => {
                    had_input = true;
                    self.update_current_dir_from_osc(&data);
                    self.append_vt_log(&data);
                    self.processor.advance(&mut self.term, &data);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    pty_closed = true;
                    break;
                }
            }
        }
        ProcessInputResult {
            had_input,
            pty_closed,
        }
    }

    /// Write user input to the PTY.
    pub fn write_to_pty(&mut self, data: &[u8]) {
        if let Ok(mut writer) = self.pty_writer.lock() {
            let _ = writer.write_all(data);
        }
        
        // Log input
        let mut log_str = String::new();
        for &b in data {
             match b {
                b'\n' => log_str.push_str("\\n"),
                b'\r' => log_str.push_str("\\r"),
                b'\t' => log_str.push_str("\\t"),
                0x1b => log_str.push_str("\\x1b"),
                0x20..=0x7e => log_str.push(b as char),
                _ => log_str.push_str(&format!("\\x{:02X}", b)),
            }
        }
        self.vt_lines.push_back(VtLogEntry::Input(log_str));
         while self.vt_lines.len() > VT_LOG_MAX_LINES {
            self.vt_lines.pop_front();
        }
    }

    /// Resize both the terminal grid and the underlying PTY.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let dims = TermDims {
            cols: cols as usize,
            rows: rows as usize,
        };
        self.term.resize(dims);
        if let Ok(mut writer) = self.pty_writer.lock() {
            let _ = writer.resize(PtySize { rows, cols });
        }
    }

    pub fn is_alive(&self) -> bool {
        if let Ok(writer) = self.pty_writer.lock() {
            writer.is_alive()
        } else {
            false
        }
    }

    /// Get a reference to the underlying Term for rendering.
    pub fn term(&self) -> &Term<VoidListener> {
        &self.term
    }

    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }

    pub fn cols(&self) -> usize {
        self.term.columns()
    }

    pub fn current_dir(&self) -> &str {
        &self.current_dir
    }

    pub fn is_bracketed_paste_enabled(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn is_focus_in_out_enabled(&self) -> bool {
        self.term.mode().contains(TermMode::FOCUS_IN_OUT)
    }

    pub fn vt_log_lines_len(&self) -> usize {
        self.vt_lines.len() + if self.vt_pending.is_empty() { 0 } else { 1 }
    }

    pub fn vt_log_line(&self, index: usize) -> Option<VtLogEntry> {
        if index < self.vt_lines.len() {
            return self.vt_lines.get(index).cloned();
        }
        if !self.vt_pending.is_empty() && index == self.vt_lines.len() {
            return Some(VtLogEntry::Output(self.vt_pending.clone()));
        }
        None
    }

    fn append_vt_log(&mut self, data: &[u8]) {
        if let Ok(text) = std::str::from_utf8(data) {
            for ch in text.chars() {
                self.push_vt_char(ch);
            }
        } else {
            for &byte in data {
                self.push_vt_byte(byte);
            }
        }
    }

    fn push_vt_char(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.vt_pending.push_str("\\n");
                self.push_vt_line();
            }
            '\r' => self.vt_pending.push_str("\\r"),
            '\t' => self.vt_pending.push_str("\\t"),
            '\u{1b}' => self.vt_pending.push_str("\\x1b"),
            c if c.is_control() => {
                let code = c as u32;
                self.vt_pending.push_str(&format!("\\u{{{:04X}}}", code));
            }
            _ => self.vt_pending.push(ch),
        }
    }

    fn push_vt_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.vt_pending.push_str("\\n");
                self.push_vt_line();
            }
            b'\r' => self.vt_pending.push_str("\\r"),
            b'\t' => self.vt_pending.push_str("\\t"),
            0x1b => self.vt_pending.push_str("\\x1b"),
            0x20..=0x7e => self.vt_pending.push(byte as char),
            _ => self.vt_pending.push_str(&format!("\\x{:02X}", byte)),
        }
    }

    fn push_vt_line(&mut self) {
        let line = std::mem::take(&mut self.vt_pending);
        self.vt_lines.push_back(VtLogEntry::Output(line));
        while self.vt_lines.len() > VT_LOG_MAX_LINES {
            self.vt_lines.pop_front();
        }
    }

    fn update_current_dir_from_osc(&mut self, data: &[u8]) {
        self.osc_tracking_buffer.extend_from_slice(data);
        let mut cursor = 0usize;

        loop {
            let slice = &self.osc_tracking_buffer[cursor..];
            let Some(rel_start) = find_subslice(slice, CWD_OSC_PREFIX) else {
                let remaining = &self.osc_tracking_buffer[cursor..];
                let keep = trailing_partial_marker_len(remaining, CWD_OSC_PREFIX);
                self.osc_tracking_buffer =
                    remaining[remaining.len().saturating_sub(keep)..].to_vec();
                return;
            };

            let start_idx = cursor + rel_start;
            let content_start = start_idx + CWD_OSC_PREFIX.len();
            let after_start = &self.osc_tracking_buffer[content_start..];

            let (end_idx, terminator_len) =
                if let Some(rel_bel) = after_start.iter().position(|&b| b == OSC_BEL) {
                    (content_start + rel_bel, 1)
                } else if let Some(rel_st) = find_subslice(after_start, OSC_ST) {
                    (content_start + rel_st, OSC_ST.len())
                } else {
                    self.osc_tracking_buffer = self.osc_tracking_buffer[start_idx..].to_vec();
                    return;
                };

            let cwd_bytes = &self.osc_tracking_buffer[content_start..end_idx];
            if !cwd_bytes.is_empty() {
                self.current_dir = String::from_utf8_lossy(cwd_bytes).to_string();
            }

            cursor = end_idx + terminator_len;
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn trailing_partial_marker_len(data: &[u8], marker: &[u8]) -> usize {
    if data.is_empty() || marker.is_empty() {
        return 0;
    }
    let max = data.len().min(marker.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if data[data.len() - len..] == marker[..len] {
            return len;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Terminal rendering (egui)
// ---------------------------------------------------------------------------

fn term_color_to_egui(color: &TermColor, is_fg: bool) -> egui::Color32 {
    match color {
        TermColor::Named(named) => named_color_to_egui(named, is_fg),
        TermColor::Spec(rgb) => egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        TermColor::Indexed(idx) => indexed_color_to_egui(*idx, is_fg),
    }
}

fn named_color_to_egui(named: &NamedColor, is_fg: bool) -> egui::Color32 {
    match named {
        NamedColor::Black => egui::Color32::from_rgb(0, 0, 0),
        NamedColor::Red => egui::Color32::from_rgb(204, 0, 0),
        NamedColor::Green => egui::Color32::from_rgb(78, 154, 6),
        NamedColor::Yellow => egui::Color32::from_rgb(196, 160, 0),
        NamedColor::Blue => egui::Color32::from_rgb(52, 101, 164),
        NamedColor::Magenta => egui::Color32::from_rgb(117, 80, 123),
        NamedColor::Cyan => egui::Color32::from_rgb(6, 152, 154),
        NamedColor::White => egui::Color32::from_rgb(211, 215, 207),
        NamedColor::BrightBlack => egui::Color32::from_rgb(85, 87, 83),
        NamedColor::BrightRed => egui::Color32::from_rgb(239, 41, 41),
        NamedColor::BrightGreen => egui::Color32::from_rgb(138, 226, 52),
        NamedColor::BrightYellow => egui::Color32::from_rgb(252, 233, 79),
        NamedColor::BrightBlue => egui::Color32::from_rgb(114, 159, 207),
        NamedColor::BrightMagenta => egui::Color32::from_rgb(173, 127, 168),
        NamedColor::BrightCyan => egui::Color32::from_rgb(52, 226, 226),
        NamedColor::BrightWhite => egui::Color32::from_rgb(238, 238, 236),
        NamedColor::Foreground | NamedColor::BrightForeground => {
            egui::Color32::from_rgb(204, 204, 204)
        }
        NamedColor::Background => egui::Color32::from_rgb(18, 18, 18),
        NamedColor::Cursor => egui::Color32::from_rgb(204, 204, 204),
        _ => {
            if is_fg {
                egui::Color32::from_rgb(204, 204, 204)
            } else {
                egui::Color32::TRANSPARENT
            }
        }
    }
}

fn indexed_color_to_egui(idx: u8, _is_fg: bool) -> egui::Color32 {
    // Standard 16 colors
    static ANSI_COLORS: [[u8; 3]; 16] = [
        [0, 0, 0],
        [204, 0, 0],
        [78, 154, 6],
        [196, 160, 0],
        [52, 101, 164],
        [117, 80, 123],
        [6, 152, 154],
        [211, 215, 207],
        [85, 87, 83],
        [239, 41, 41],
        [138, 226, 52],
        [252, 233, 79],
        [114, 159, 207],
        [173, 127, 168],
        [52, 226, 226],
        [238, 238, 236],
    ];
    if (idx as usize) < 16 {
        let c = ANSI_COLORS[idx as usize];
        return egui::Color32::from_rgb(c[0], c[1], c[2]);
    }
    // 216 color cube (indices 16-231)
    if idx < 232 {
        let idx = idx - 16;
        let r = (idx / 36) % 6;
        let g = (idx / 6) % 6;
        let b = idx % 6;
        let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
        return egui::Color32::from_rgb(to_val(r), to_val(g), to_val(b));
    }
    // Grayscale ramp (indices 232-255)
    let v = 8 + 10 * (idx - 232);
    egui::Color32::from_rgb(v, v, v)
}

fn align_to_pixels(value: f32, pixels_per_point: f32) -> f32 {
    if pixels_per_point <= 0.0 {
        return value;
    }
    (value * pixels_per_point).round() / pixels_per_point
}

fn align_to_pixels_ceil(value: f32, pixels_per_point: f32) -> f32 {
    if pixels_per_point <= 0.0 {
        return value;
    }
    (value * pixels_per_point).ceil() / pixels_per_point
}

pub(crate) fn aligned_row_height(ui: &egui::Ui, font_id: &egui::FontId) -> f32 {
    let raw = ui.fonts(|f| f.row_height(font_id)).max(1.0);
    let aligned = align_to_pixels_ceil(raw, ui.ctx().pixels_per_point());
    aligned.max(1.0)
}

pub(crate) fn aligned_glyph_width(ui: &egui::Ui, font_id: &egui::FontId, ch: char) -> f32 {
    let raw = ui.fonts(|f| f.glyph_width(font_id, ch));
    if raw <= 0.0 {
        return 0.0;
    }
    align_to_pixels(raw, ui.ctx().pixels_per_point())
}

pub fn render_terminal(
    ui: &mut egui::Ui,
    terminal: Option<&TerminalInstance>,
    selection_state: &mut TerminalSelectionState,
    input_blocked: bool,
    scroll_request: Option<ScrollRequest>,
    scroll_id: u64,
) -> Option<egui::Rect> {
    let terminal = match terminal {
        Some(t) => t,
        None => {
            ui.label(
                egui::RichText::new("Terminal not available.")
                    .color(egui::Color32::from_gray(120))
                    .monospace(),
            );
            return None;
        }
    };

    let term = terminal.term();
    let grid = term.grid();
    let content = term.renderable_content();
    let cursor = content.cursor;
    let num_cols = term.columns();
    let total_lines = grid.total_lines();
    let history_lines = grid.history_size();
    let top_line = -(history_lines as i32);
    let font_id = egui::FontId::monospace(TERM_FONT_SIZE);
    let pixels_per_point = ui.ctx().pixels_per_point();
    let char_width = aligned_glyph_width(ui, &font_id, 'M');
    // Set item_spacing to 0 BEFORE calculating row_height and show_rows,
    // so the scroll calculations use the same spacing as the actual layout.
    ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);
    let row_height = aligned_row_height(ui, &font_id);
    let row_height_with_spacing = row_height + ui.spacing().item_spacing.y;
    let cursor_row_idx = if total_lines == 0 {
        0
    } else {
        (cursor.point.line.0 - top_line).clamp(0, total_lines.saturating_sub(1) as i32) as usize
    };
    let cursor_col_idx = if num_cols == 0 {
        0
    } else {
        cursor.point.column.0.min(num_cols.saturating_sub(1))
    };
    let selection_range = selection_state.normalized();
    let mut ime_cursor_rect = None;

    // Cursor blink: 500ms on / 500ms off
    let cursor_visible = {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        cursor.shape != ansi::CursorShape::Hidden && (ms / 500) % 2 == 0
    };

    // Use scroll_id in the ScrollArea ID so Ctrl+L resets the scroll state
    let mut scroll = egui::ScrollArea::vertical()
        .id_source(("terminal_scroll", scroll_id))
        .auto_shrink([false, false])
        .animated(true);

    if let Some(req) = scroll_request {
        let offset = match req {
            // Show the terminal "screen" (last `screen_lines` rows), not the absolute end of the
            // scrollback buffer (which can be blank below the cursor and confusing on startup).
            ScrollRequest::ScreenTop => Some(row_height * history_lines as f32),
            // Scroll to absolute top (offset 0) - used for a clean slate
            ScrollRequest::CursorTop => Some(0.0),
            // Cursor follow is handled with viewport-aware logic below.
            ScrollRequest::CursorLine => None,
        };
        if let Some(offset) = offset {
            let offset = align_to_pixels_ceil(offset, pixels_per_point).max(0.0);
            scroll = scroll.vertical_scroll_offset(offset);
        }
    }

    scroll.show_viewport(ui, |ui, viewport| {
        // Compute content_height with viewport known so that scrolling to
        // ScreenTop (history_lines * row_height) fully hides scrollback.
        // Without this, the remainder (viewport_h - screen_lines * row_height)
        // causes a partial scrollback row to "leak" at the top after Ctrl+L.
        let natural =
            (row_height_with_spacing * total_lines as f32 - ui.spacing().item_spacing.y).max(0.0);
        let content_height = natural.max(row_height * history_lines as f32 + viewport.height());
        ui.set_height(content_height);

        if matches!(scroll_request, Some(ScrollRequest::CursorLine)) {
            let cursor_top = cursor_row_idx as f32 * row_height_with_spacing;
            let cursor_bottom = cursor_top + row_height;
            let cursor_above = cursor_top < viewport.min.y;
            let cursor_below = cursor_bottom > viewport.max.y;

            // Only scroll when the cursor is outside the visible range.
            if cursor_above || cursor_below {
                let target_rect = egui::Rect::from_min_size(
                    egui::pos2(ui.min_rect().left(), ui.min_rect().top() + cursor_top),
                    egui::vec2(1.0, row_height),
                );
                ui.scroll_to_rect(target_rect, Some(egui::Align::BOTTOM));
            }
        }

        let mut min_row = (viewport.min.y / row_height_with_spacing).floor().max(0.0) as usize;
        let mut max_row = (viewport.max.y / row_height_with_spacing).ceil().max(0.0) as usize + 1;

        if min_row > total_lines {
            min_row = total_lines;
        }
        if max_row > total_lines {
            max_row = total_lines;
        }
        if min_row > max_row {
            min_row = max_row;
        }

        let viewport_rect = egui::Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), ui.max_rect().top() + viewport.min.y),
            egui::pos2(ui.max_rect().right(), ui.max_rect().top() + viewport.max.y),
        );
        let text_grid_max_x = viewport_rect.left() + char_width * num_cols as f32;
        if total_lines > 0 && num_cols > 0 && char_width > 0.0 && row_height > 0.0 {
            let cursor_x = viewport_rect.left() + cursor_col_idx as f32 * char_width;
            let cursor_y = ui.max_rect().top() + cursor_row_idx as f32 * row_height_with_spacing;
            ime_cursor_rect = Some(egui::Rect::from_min_size(
                egui::pos2(cursor_x, cursor_y),
                egui::vec2(char_width.max(1.0), row_height.max(1.0)),
            ));
        }
        let to_cell = |pos: egui::Pos2| -> Option<(usize, usize)> {
            if total_lines == 0 || num_cols == 0 || char_width <= 0.0 {
                return None;
            }
            if !viewport_rect.contains(pos) {
                return None;
            }
            // Ignore clicks in the right gutter / scrollbar region.
            if pos.x >= text_grid_max_x {
                return None;
            }

            let y = (pos.y - ui.max_rect().top()).max(0.0);
            let mut row = (y / row_height_with_spacing).floor() as usize;
            if row >= total_lines {
                row = total_lines - 1;
            }

            let x = (pos.x - viewport_rect.left()).max(0.0);
            let mut col = (x / char_width).floor() as usize;
            if col >= num_cols {
                col = num_cols - 1;
            }

            Some((row, col))
        };

        if !input_blocked {
            ui.input(|i| {
                let pointer = &i.pointer;

                if pointer.button_pressed(egui::PointerButton::Primary) {
                    if let Some((row, col)) = pointer.interact_pos().and_then(to_cell) {
                        selection_state.start(row, col);
                    }
                }

                if selection_state.dragging && pointer.button_down(egui::PointerButton::Primary) {
                    if let Some((row, col)) = pointer.interact_pos().and_then(to_cell) {
                        selection_state.update(row, col);
                    }
                }

                if pointer.button_released(egui::PointerButton::Primary)
                    && selection_state.dragging
                {
                    if let Some((row, col)) = pointer.interact_pos().and_then(to_cell) {
                        selection_state.update(row, col);
                    }
                    if !selection_state.has_selection() {
                        selection_state.clear();
                    }
                    selection_state.stop_dragging();
                }
            });
        } else if selection_state.dragging {
            selection_state.stop_dragging();
        }

        let row_layout =
            egui::Layout::left_to_right(egui::Align::Min).with_cross_align(egui::Align::Min);
        let row_start = min_row;

        let y_min = ui.max_rect().top() + min_row as f32 * row_height_with_spacing;
        let y_max = ui.max_rect().top() + max_row as f32 * row_height_with_spacing;
        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), y_min..=y_max);

        ui.allocate_ui_at_rect(rect, |viewport_ui| {
            let row_width = viewport_ui.max_rect().width();
            let base_left = viewport_ui.min_rect().left();
            let base_top = align_to_pixels(viewport_ui.min_rect().top(), pixels_per_point);
            for row_idx in min_row..max_row {
                let line = Line(top_line + row_idx as i32);
                let row = &grid[line];
                let mut job = egui::text::LayoutJob::default();

                for col_idx in 0..num_cols {
                    let col = Column(col_idx);
                    let cell = &row[col];
                    let ch = cell.c;
                    let display_char = if ch == '\0' || ch == ' ' { ' ' } else { ch };

                    let show_cursor = cursor.point == Point::new(line, col) && cursor_visible;
                    let is_wide_continuation = cell.flags.contains(CellFlags::WIDE_CHAR_SPACER);
                    if is_wide_continuation {
                        continue;
                    }
                    let is_selected = selection_range_contains(selection_range, row_idx, col_idx);

                    let is_ghost = cell.flags.intersects(CellFlags::DIM | CellFlags::ITALIC);
                    let is_inverse = cell.flags.contains(CellFlags::INVERSE);

                    // Base colors (before selection/cursor override)
                    let (mut base_fg, mut base_bg) = if is_ghost {
                        (egui::Color32::from_gray(140), egui::Color32::TRANSPARENT)
                    } else {
                        let f = term_color_to_egui(&cell.fg, true);
                        let b = term_color_to_egui(&cell.bg, false);
                        (f, b)
                    };

                    // Handle SGR 7 (reverse video): swap fg and bg
                    if is_inverse {
                        if base_bg == egui::Color32::TRANSPARENT {
                            base_bg = egui::Color32::from_rgb(18, 18, 18);
                        }
                        std::mem::swap(&mut base_fg, &mut base_bg);
                    }

                    let fg = if show_cursor {
                        egui::Color32::from_rgb(18, 18, 18)
                    } else if is_selected {
                        egui::Color32::from_rgb(18, 18, 18)
                    } else {
                        base_fg
                    };
                    let bg = if is_selected {
                        egui::Color32::from_rgb(180, 180, 180)
                    } else if show_cursor {
                        egui::Color32::from_rgb(204, 204, 204)
                    } else {
                        base_bg
                    };

                    let text_format = egui::TextFormat {
                        font_id: font_id.clone(),
                        color: fg,
                        background: bg,
                        ..Default::default()
                    };
                    job.append(&display_char.to_string(), 0.0, text_format);
                }

                let row_top = base_top + (row_idx - row_start) as f32 * row_height_with_spacing;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(base_left, row_top),
                    egui::vec2(row_width, row_height),
                );

                viewport_ui.allocate_ui_at_rect(rect, |row_ui| {
                    row_ui.with_layout(row_layout, |row_ui| {
                        let label = egui::Label::new(job).wrap(false);
                        row_ui.add(label);
                    });
                });
            }
        });
    });

    ime_cursor_rect
}

pub fn selected_text_for_copy(
    terminal: &TerminalInstance,
    selection_state: &TerminalSelectionState,
) -> Option<String> {
    if !selection_state.has_selection() {
        return None;
    }
    selected_text(terminal.term(), selection_state)
}

fn selection_range_contains(
    range: Option<((usize, usize), (usize, usize))>,
    row: usize,
    col: usize,
) -> bool {
    let Some(((start_row, start_col), (end_row, end_col))) = range else {
        return false;
    };

    if row < start_row || row > end_row {
        return false;
    }
    if start_row == end_row {
        return row == start_row && col >= start_col && col <= end_col;
    }
    if row == start_row {
        return col >= start_col;
    }
    if row == end_row {
        return col <= end_col;
    }
    true
}

fn selected_text(term: &Term<VoidListener>, selection_state: &TerminalSelectionState) -> Option<String> {
    let ((start_row, start_col), (end_row, end_col)) = selection_state.normalized()?;
    if start_row == end_row && start_col == end_col {
        return None;
    }

    let grid = term.grid();
    let total_lines = grid.total_lines();
    let num_cols = term.columns();
    if total_lines == 0 || num_cols == 0 || start_row >= total_lines {
        return None;
    }

    let history_lines = grid.history_size();
    let top_line = -(history_lines as i32);
    let last_row = end_row.min(total_lines - 1);
    let selected_rows = last_row.saturating_sub(start_row) + 1;
    let estimated = selected_rows.saturating_mul(num_cols.saturating_add(1));
    let reserve = estimated.min(MAX_SELECTION_COPY_BYTES);
    let mut out = String::with_capacity(reserve);

    'rows: for row_idx in start_row..=last_row {
        if out.len() >= MAX_SELECTION_COPY_BYTES {
            break;
        }
        let line = Line(top_line + row_idx as i32);
        let row = &grid[line];
        let line_start = if row_idx == start_row { start_col } else { 0 };
        let line_end = if row_idx == last_row {
            end_col.min(num_cols - 1)
        } else {
            num_cols - 1
        };

        if line_start > line_end {
            continue;
        }

        let row_start_len = out.len();
        let mut row_non_space_len = 0usize;
        for col_idx in line_start..=line_end {
            let cell = &row[Column(col_idx)];
            if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            let ch_len = ch.len_utf8();
            if out.len().saturating_add(ch_len) > MAX_SELECTION_COPY_BYTES {
                out.truncate(row_start_len + row_non_space_len);
                break 'rows;
            }
            out.push(ch);
            if ch != ' ' {
                row_non_space_len = out.len() - row_start_len;
            }
        }
        out.truncate(row_start_len + row_non_space_len);

        if row_idx != last_row {
            if out.len().saturating_add(1) > MAX_SELECTION_COPY_BYTES {
                break;
            }
            out.push('\n');
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn render_vt_log(ui: &mut egui::Ui, terminal: Option<&TerminalInstance>) {
    let terminal = match terminal {
        Some(t) => t,
        None => {
            ui.label(
                egui::RichText::new("VT log not available.")
                    .color(egui::Color32::from_gray(120))
                    .monospace(),
            );
            return;
        }
    };

    let total_lines = terminal.vt_log_lines_len();
    let font_id = egui::FontId::monospace(12.0);
    // Rough estimate of row height
    let row_height = ui.fonts(|f| f.row_height(&font_id));

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, total_lines, |ui, row_range| {
            // Use tighter spacing
            ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 2.0);
            for row_idx in row_range {
                let Some(entry) = terminal.vt_log_line(row_idx) else {
                    continue;
                };
                
                let (text, color, icon) = match &entry {
                    VtLogEntry::Input(s) => (s, egui::Color32::from_rgb(100, 200, 100), "➜"),
                    VtLogEntry::Output(s) => (s, egui::Color32::from_gray(170), " "),
                };
                
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(icon)
                            .monospace()
                            .size(12.0)
                            .color(if matches!(entry, VtLogEntry::Input(_)) {
                                egui::Color32::from_rgb(100, 200, 100)
                            } else {
                                egui::Color32::TRANSPARENT // Output: invisible icon just for spacing? or empty string.
                            })
                    );
                    
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .monospace()
                                .color(color)
                        ).wrap(false)
                    );
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Keyboard input → PTY bytes
// ---------------------------------------------------------------------------

pub fn key_to_terminal_input(
    event: &winit::event::KeyEvent,
    modifiers: &winit::event::Modifiers,
) -> Option<Vec<u8>> {
    if !event.state.is_pressed() {
        return None;
    }

    let ctrl = modifiers.state().control_key();

    // Ctrl + letter → control character (0x01..=0x1a)
    if ctrl {
        if let Key::Character(text) = &event.logical_key {
            let ch = text.chars().next()?;
            if ch.is_ascii_alphabetic() {
                let ctrl_byte = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                return Some(vec![ctrl_byte]);
            }
        }
    }

    // Handle named (special) keys
    match &event.logical_key {
        Key::Named(named) => {
            let bytes: &[u8] = match named {
                NamedKey::Enter => b"\r",
                NamedKey::Backspace => b"\x7f",
                NamedKey::Tab => b"\t",
                NamedKey::Escape => b"\x1b",
                NamedKey::Space => b" ",
                NamedKey::ArrowUp => b"\x1b[A",
                NamedKey::ArrowDown => b"\x1b[B",
                NamedKey::ArrowRight => b"\x1b[C",
                NamedKey::ArrowLeft => b"\x1b[D",
                NamedKey::Home => b"\x1b[H",
                NamedKey::End => b"\x1b[F",
                NamedKey::PageUp => b"\x1b[5~",
                NamedKey::PageDown => b"\x1b[6~",
                NamedKey::Insert => b"\x1b[2~",
                NamedKey::Delete => b"\x1b[3~",
                NamedKey::F1 => b"\x1bOP",
                NamedKey::F2 => b"\x1bOQ",
                NamedKey::F3 => b"\x1bOR",
                NamedKey::F4 => b"\x1bOS",
                NamedKey::F5 => b"\x1b[15~",
                NamedKey::F6 => b"\x1b[17~",
                NamedKey::F7 => b"\x1b[18~",
                NamedKey::F8 => b"\x1b[19~",
                NamedKey::F9 => b"\x1b[20~",
                NamedKey::F10 => b"\x1b[21~",
                NamedKey::F11 => b"\x1b[23~",
                NamedKey::F12 => b"\x1b[24~",
                _ => return None,
            };
            Some(bytes.to_vec())
        }
        Key::Character(text) => {
            if let Some(ref text) = event.text {
                Some(text.as_bytes().to_vec())
            } else {
                Some(text.as_bytes().to_vec())
            }
        }
        _ => None,
    }
}
