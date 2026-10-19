//! Embedded terminals: a PTY per session, parsed by `vt100`, painted with egui.

use crate::theme::Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Id, Key, Modifiers, Sense, vec2};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct TerminalSession {
    pub uid: u64,
    pub name: String,
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pub exited: Option<String>,
    size: (u16, u16),
    scroll_offset: usize,
}

pub struct SpawnSpec {
    pub name: String,
    /// `None` runs the interactive shell.
    pub program: Option<(PathBuf, Vec<String>)>,
    pub cwd: Option<PathBuf>,
    pub shell: String,
    pub env: Vec<String>,
    pub scrollback: usize,
}

impl TerminalSession {
    pub fn spawn(spec: SpawnSpec, repaint: egui::Context) -> Result<Self, String> {
        static UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let (rows, cols) = (24u16, 100u16);
        let pty = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("openpty: {e}"))?;
        let mut cmd = match &spec.program {
            Some((prog, args)) => {
                let mut c = CommandBuilder::new(prog);
                c.args(args);
                c
            }
            None => {
                let mut c = CommandBuilder::new(&spec.shell);
                // Login shell so PATH matches the user's terminal (GUI apps get a minimal PATH).
                c.arg("-l");
                c
            }
        };
        if let Some(cwd) = spec.cwd.as_ref().filter(|p| p.is_dir()) {
            cmd.cwd(cwd);
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Ludo");
        for kv in &spec.env {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k.trim(), v.trim());
            }
        }
        let child = pty.slave.spawn_command(cmd).map_err(|e| format!("spawn: {e}"))?;
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pty.master.take_writer().map_err(|e| e.to_string())?;
        let rows_nz = std::num::NonZeroU16::new(rows).unwrap();
        let cols_nz = std::num::NonZeroU16::new(cols).unwrap();
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows_nz, cols_nz, spec.scrollback)));
        let p2 = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut p) = p2.lock() {
                            p.process(&buf[..n]);
                        }
                        repaint.request_repaint();
                    }
                }
            }
            repaint.request_repaint();
        });
        Ok(Self {
            uid: UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name: spec.name,
            parser,
            master: pty.master,
            writer,
            child,
            exited: None,
            size: (rows, cols),
            scroll_offset: 0,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.scroll_offset = 0;
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn poll_exit(&mut self) {
        if self.exited.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exited = Some(format!("process exited with code {}", status.exit_code()));
            }
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == self.size || rows == 0 || cols == 0 {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(std::num::NonZeroU16::new(rows).unwrap(), std::num::NonZeroU16::new(cols).unwrap());
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    pub fn cwd_hint(path: &Path) -> PathBuf {
        if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.kill();
    }
}

fn ansi_color(c: vt100::Color, default: Color32, p: &Palette) -> Color32 {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) if i < 16 => p.ansi[i as usize],
        vt100::Color::Idx(i) if i >= 232 => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
        vt100::Color::Idx(i) => {
            let i = i - 16;
            let lvl = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Color32::from_rgb(lvl(i / 36), lvl((i / 6) % 6), lvl(i % 6))
        }
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

fn key_bytes(key: Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.ctrl {
        let name = key.name();
        if name.len() == 1 {
            let ch = name.as_bytes()[0].to_ascii_lowercase();
            if ch.is_ascii_lowercase() {
                return Some(vec![ch - b'a' + 1]);
            }
        }
        match key {
            Key::OpenBracket => return Some(vec![0x1b]),
            Key::Backslash => return Some(vec![0x1c]),
            Key::CloseBracket => return Some(vec![0x1d]),
            _ => {}
        }
    }
    let arrow = |c: u8| {
        if app_cursor { vec![0x1b, b'O', c] } else { vec![0x1b, b'[', c] }
    };
    let alt_prefix = |mut v: Vec<u8>| {
        if mods.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    Some(match key {
        Key::Enter => vec![b'\r'],
        Key::Backspace => alt_prefix(vec![0x7f]),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        Key::ArrowUp => arrow(b'A'),
        Key::ArrowDown => arrow(b'B'),
        // Option+arrows jump words like in macOS terminals.
        Key::ArrowRight if mods.alt => b"\x1bf".to_vec(),
        Key::ArrowLeft if mods.alt => b"\x1bb".to_vec(),
        Key::ArrowRight => arrow(b'C'),
        Key::ArrowLeft => arrow(b'D'),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    })
}

pub fn show(ui: &mut egui::Ui, term: &mut TerminalSession, p: &Palette, font_size: f32) {
    term.poll_exit();
    let font = FontId::new(font_size, FontFamily::Monospace);
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let id = Id::unique(("terminal", term.uid));
    let avail = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(avail, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let rows = ((rect.height() - 8.0) / row_h).floor().max(2.0) as u16;
    let cols = ((rect.width() - 12.0) / char_w).floor().max(10.0) as u16;
    term.resize(rows, cols);

    if response.clicked() {
        response.request_focus();
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
        let app_cursor = term.parser.lock().map(|p| p.screen().application_cursor()).unwrap_or(false);
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Text(t) => term.write(t.as_bytes()),
                egui::Event::Paste(t) => term.write(t.replace("\r\n", "\n").as_bytes()),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.mac_cmd {
                        continue; // leave Cmd shortcuts to the app
                    }
                    if let Some(b) = key_bytes(key, modifiers, app_cursor) {
                        term.write(&b);
                    }
                }
                _ => {}
            }
        }
    }
    if response.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy.abs() > 0.0 {
            let lines = (dy / row_h).round() as i64;
            let max = term.parser.lock().map(|p| p.screen().scrollback_capacity()).unwrap_or(0) as i64;
            term.scroll_offset = (term.scroll_offset as i64 + lines).clamp(0, max) as usize;
        }
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, p.bg);
    let Ok(mut parser) = term.parser.lock() else { return };
    parser.screen_mut().set_scrollback(term.scroll_offset);
    term.scroll_offset = parser.screen().scrollback();
    let screen = parser.screen();
    let (srows, scols) = screen.size();
    let origin = rect.min + vec2(6.0, 4.0);
    for r in 0..srows.get() {
        let mut job = LayoutJob::default();
        let y = origin.y + r as f32 * row_h;
        let mut c = 0;
        while c < scols.get() {
            let Some(cell) = screen.cell(r, c) else { break };
            let mut fg = ansi_color(cell.fgcolor(), p.text, p);
            let mut bg = ansi_color(cell.bgcolor(), Color32::TRANSPARENT, p);
            if cell.bold() {
                if let vt100::Color::Idx(i) = cell.fgcolor() {
                    if i < 8 {
                        fg = p.ansi[i as usize + 8];
                    }
                }
            }
            if cell.inverse() {
                let fg_old = fg;
                fg = if bg == Color32::TRANSPARENT { p.bg } else { bg };
                bg = fg_old;
            }
            if cell.dim() {
                fg = fg.gamma_multiply(0.6);
            }
            if bg != Color32::TRANSPARENT {
                let x = origin.x + c as f32 * char_w;
                let w = if cell.is_wide() { 2.0 } else { 1.0 } * char_w;
                painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x, y), vec2(w, row_h)), 0.0, bg);
            }
            if cell.is_wide_continuation() {
                c += 1;
                continue;
            }
            let s = if cell.has_contents() { cell.contents() } else { " " };
            let mut fmt = TextFormat::simple(font.clone(), fg);
            fmt.italics = cell.italic();
            if cell.underline() {
                fmt.underline = egui::Stroke::new(1.0, fg);
            }
            job.append(s, 0.0, fmt);
            c += 1;
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(egui::pos2(origin.x, y), galley, p.text);
    }
    if !screen.hide_cursor() && term.scroll_offset == 0 {
        let (cr, cc) = screen.cursor_position();
        let cursor = egui::Rect::from_min_size(
            egui::pos2(origin.x + cc as f32 * char_w, origin.y + cr as f32 * row_h),
            vec2(char_w, row_h),
        );
        if focused {
            painter.rect_filled(cursor, 0.0, p.text.gamma_multiply(0.5));
        } else {
            painter.rect_stroke(cursor, 0.0, egui::Stroke::new(1.0, p.text_dim), egui::StrokeKind::Inside);
        }
    }
    drop(parser);
    if let Some(msg) = &term.exited {
        painter.text(
            rect.right_bottom() - vec2(8.0, 6.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("[{msg}]"),
            FontId::new(font_size * 0.9, FontFamily::Monospace),
            p.text_dim,
        );
    }
}
