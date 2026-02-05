use std::collections::VecDeque;
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{self, Color as TermColor, NamedColor};

use winit::keyboard::{Key, NamedKey};

use crate::pty::{self, PtySize, PtyWriter};

pub const TERM_FONT_SIZE: f32 = 14.0;
const VT_LOG_MAX_LINES: usize = 2000;

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
    vt_lines: VecDeque<String>,
    vt_pending: String,
    _reader_thread: thread::JoinHandle<()>,
}

impl TerminalInstance {
    pub fn new(rows: u16, cols: u16) -> io::Result<Self> {
        let size = PtySize { rows, cols };
        let (mut reader, writer) = pty::spawn_pty(size)?;
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
            _reader_thread: reader_thread,
        })
    }

    /// Process pending PTY output, feeding bytes into the terminal emulator.
    pub fn process_input(&mut self) {
        while let Ok(data) = self.rx.try_recv() {
            self.append_vt_log(&data);
            self.processor.advance(&mut self.term, &data);
        }
    }

    /// Write user input to the PTY.
    pub fn write_to_pty(&self, data: &[u8]) {
        if let Ok(mut writer) = self.pty_writer.lock() {
            let _ = writer.write_all(data);
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

    pub fn vt_log_lines_len(&self) -> usize {
        self.vt_lines.len() + if self.vt_pending.is_empty() { 0 } else { 1 }
    }

    pub fn vt_log_line(&self, index: usize) -> Option<&str> {
        if index < self.vt_lines.len() {
            return self.vt_lines.get(index).map(|line| line.as_str());
        }
        if !self.vt_pending.is_empty() && index == self.vt_lines.len() {
            return Some(self.vt_pending.as_str());
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
        self.vt_lines.push_back(line);
        while self.vt_lines.len() > VT_LOG_MAX_LINES {
            self.vt_lines.pop_front();
        }
    }
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

fn indexed_color_to_egui(idx: u8, is_fg: bool) -> egui::Color32 {
    // Standard 16 colors
    static ANSI_COLORS: [[u8; 3]; 16] = [
        [0, 0, 0],       [204, 0, 0],     [78, 154, 6],   [196, 160, 0],
        [52, 101, 164],   [117, 80, 123],  [6, 152, 154],  [211, 215, 207],
        [85, 87, 83],     [239, 41, 41],   [138, 226, 52], [252, 233, 79],
        [114, 159, 207],  [173, 127, 168], [52, 226, 226], [238, 238, 236],
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

pub fn render_terminal(ui: &mut egui::Ui, terminal: Option<&TerminalInstance>) {
    let terminal = match terminal {
        Some(t) => t,
        None => {
            ui.label(
                egui::RichText::new("Terminal not available.")
                    .color(egui::Color32::from_gray(120))
                    .monospace(),
            );
            return;
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
    let row_height = ui.fonts(|f| f.row_height(&font_id)).max(1.0);

    // Cursor blink: 500ms on / 500ms off
    let cursor_visible = {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        (ms / 500) % 2 == 0
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, total_lines, |ui, row_range| {
            ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);
            for row_idx in row_range {
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

                    let fg = if show_cursor {
                        egui::Color32::from_rgb(18, 18, 18)
                    } else {
                        term_color_to_egui(&cell.fg, true)
                    };
                    let bg = if show_cursor {
                        egui::Color32::from_rgb(204, 204, 204)
                    } else {
                        let bg_color = term_color_to_egui(&cell.bg, false);
                        if bg_color == egui::Color32::TRANSPARENT {
                            egui::Color32::TRANSPARENT
                        } else {
                            bg_color
                        }
                    };

                    let text_format = egui::TextFormat {
                        font_id: font_id.clone(),
                        color: fg,
                        background: bg,
                        ..Default::default()
                    };
                    job.append(&display_char.to_string(), 0.0, text_format);
                }
                ui.label(job);
            }
        });
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
    let row_height = ui.fonts(|f| f.row_height(&font_id)).max(1.0);

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, total_lines, |ui, row_range| {
            ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);
            for row_idx in row_range {
                let Some(line) = terminal.vt_log_line(row_idx) else {
                    continue;
                };
                let label = egui::Label::new(
                    egui::RichText::new(line)
                        .monospace()
                        .color(egui::Color32::from_gray(170)),
                )
                .wrap(false);
                ui.add(label);
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
