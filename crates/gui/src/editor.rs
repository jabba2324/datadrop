//! The SQL editor: a document per tab, the highlighted text area with line numbers,
//! and find / replace.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use datadrop_core::QueryOutput;
use datadrop_core::sql::{self, Dialect};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{FontId, Galley, TextBuffer};

use crate::backend::Task;
use crate::theme;

/// One editor tab.
pub struct Document {
    /// Unique for the app's lifetime: the tab's widget ids and its database session.
    pub id: u64,
    pub path: Option<PathBuf>,
    /// For untitled tabs: "Untitled 3".
    pub untitled_name: String,
    pub text: String,
    /// The text as last saved (or opened), to know whether there are changes.
    pub saved_text: String,
    pub profile: Option<String>,
    pub output: Option<QueryOutput>,
    /// Which result set is shown when a script returned several.
    pub shown_result: usize,
    pub running: Option<Task>,
    /// Exporting a result to a file.
    pub exporting: Option<Task>,
    pub run_started: Option<Instant>,
    /// Select this byte range in the editor on the next frame (find, say).
    pub select: Option<Range<usize>>,
    /// Scroll so this byte offset is visible on the next frame.
    pub scroll_to: Option<usize>,
    pub request_focus: bool,
    layout_cache: Option<(u64, Arc<Galley>)>,
    /// Where the cursor was last frame (char index), to scroll when it moves.
    last_cursor: Option<usize>,
}

impl Document {
    pub fn untitled(id: u64, n: usize, profile: Option<String>) -> Self {
        Self {
            id,
            path: None,
            untitled_name: format!("Untitled {n}"),
            text: String::new(),
            saved_text: String::new(),
            profile,
            output: None,
            shown_result: 0,
            running: None,
            exporting: None,
            run_started: None,
            select: None,
            scroll_to: None,
            request_focus: true,
            layout_cache: None,
            last_cursor: None,
        }
    }

    pub fn open(id: u64, path: &Path, profile: Option<String>) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "the file isn't UTF-8 text")
        })?;
        // Keep line endings as they are; egui handles \r\n.
        let mut doc = Self::untitled(id, 0, profile);
        doc.path = Some(path.to_owned());
        doc.saved_text = text.clone();
        doc.text = text;
        Ok(doc)
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            None => self.untitled_name.clone(),
        }
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved_text
    }

    /// An untitled tab nobody has typed in: replaced when a file is opened.
    pub fn is_pristine(&self) -> bool {
        self.path.is_none() && self.text.is_empty() && self.output.is_none()
    }

    pub fn save_to(&mut self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("datadrop-tmp");
        std::fs::write(&tmp, self.text.as_bytes())?;
        std::fs::rename(&tmp, path)?;
        self.path = Some(path.to_owned());
        self.saved_text = self.text.clone();
        Ok(())
    }

    pub fn session(&self) -> String {
        format!("tab-{}", self.id)
    }

    pub fn editor_id(&self) -> egui::Id {
        egui::Id::new(("sql-editor", self.id))
    }

    /// The selected text's byte range, if there's a selection.
    pub fn selection(&self, ctx: &egui::Context) -> Option<Range<usize>> {
        let r = self.cursor_range(ctx)?;
        (!r.is_empty()).then_some(r)
    }

    /// The cursor (or selection) as a byte range.
    pub fn cursor_range(&self, ctx: &egui::Context) -> Option<Range<usize>> {
        let state = egui::TextEdit::load_state(ctx, self.editor_id())?;
        let r = state.cursor.char_range()?.as_sorted_char_range();
        Some(char_to_byte(&self.text, r.start.0)..char_to_byte(&self.text, r.end.0))
    }
}

pub fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

pub fn byte_to_char(text: &str, b: usize) -> usize {
    text[..b.min(text.len())].chars().count()
}

/// Find / replace state, shared by all tabs (it acts on the open one).
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub replace: bool,
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    /// Focus the search field on the next frame.
    pub focus: bool,
    /// Index into the matches of the one last moved to.
    pub current: Option<usize>,
}

impl Find {
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        if self.open {
            find_all(text, &self.query, self.match_case)
        } else {
            vec![]
        }
    }
}

/// Non-overlapping matches of `query` in `text`, as byte ranges.
pub fn find_all(text: &str, query: &str, match_case: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return vec![];
    }
    if match_case {
        return text
            .match_indices(query)
            .map(|(i, m)| i..i + m.len())
            .collect();
    }
    if query.is_ascii() {
        // ASCII-insensitive byte search: can't land inside a multi-byte char, as
        // every byte of the query is ASCII.
        let (t, q) = (text.as_bytes(), query.as_bytes());
        let mut out = vec![];
        let mut i = 0;
        while i + q.len() <= t.len() {
            if t[i..i + q.len()].eq_ignore_ascii_case(q) {
                out.push(i..i + q.len());
                i += q.len();
            } else {
                i += 1;
            }
        }
        return out;
    }
    // Unicode: compare char by char, lower-cased.
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut out = vec![];
    let mut starts = text.char_indices().map(|(i, _)| i).peekable();
    while let Some(start) = starts.next() {
        let mut lowered = text[start..].chars().flat_map(char::to_lowercase);
        if q.iter().all(|c| lowered.next() == Some(*c)) {
            // Find where the match ends in the original text.
            let mut n = 0;
            let mut end = start;
            for c in text[start..].chars() {
                if n >= q.len() {
                    break;
                }
                n += c.to_lowercase().count();
                end += c.len_utf8();
            }
            out.push(start..end);
            while starts.peek().is_some_and(|&s| s < end) {
                starts.next();
            }
        }
    }
    out
}

/// The editor for a document: line numbers, highlighted text, find matches.
pub fn editor_ui(
    ui: &mut egui::Ui,
    doc: &mut Document,
    dialect: Dialect,
    matches: &[Range<usize>],
    current: Option<usize>,
    font_size: f32,
) {
    let id = doc.editor_id();
    let font = FontId::monospace(font_size);
    let dark = ui.visuals().dark_mode;

    if let Some(sel) = doc.select.take() {
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let (a, b) = (
            byte_to_char(&doc.text, sel.start),
            byte_to_char(&doc.text, sel.end),
        );
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        egui::TextEdit::store_state(ui.ctx(), id, state);
    }

    let outer = ui.available_size();
    let bg = if dark {
        egui::Color32::from_gray(24)
    } else {
        egui::Color32::from_gray(252)
    };
    egui::Frame::new().fill(bg).show(ui, |ui| {
        ui.set_min_size(outer);
        // Line numbers scroll up and down with the text, but stay put when a long
        // line scrolls sideways.
        egui::ScrollArea::vertical()
            .id_salt(("editor-scroll", doc.id))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let lines = doc.text.split('\n').count();
                    let width = lines.to_string().len().max(2);
                    let gutter: String = (1..=lines)
                        .map(|n| format!("{n:>width$} "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let gutter_color = ui.visuals().weak_text_color().gamma_multiply(0.8);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(gutter)
                                    .font(font.clone())
                                    .color(gutter_color),
                            )
                            .selectable(false)
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                    ui.add_space(6.0);

                    let Document {
                        text,
                        layout_cache,
                        last_cursor,
                        ..
                    } = doc;
                    let mut layouter = |ui: &egui::Ui, buf: &dyn TextBuffer, _wrap: f32| {
                        let text = buf.as_str();
                        let key = {
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            (text, dark, matches, current, font_size.to_bits()).hash(&mut h);
                            h.finish()
                        };
                        if let Some((k, galley)) = layout_cache.as_ref()
                            && *k == key
                        {
                            return galley.clone();
                        }
                        let job = highlight(text, dialect, matches, current, dark, &font);
                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                        *layout_cache = Some((key, galley.clone()));
                        galley
                    };
                    let width = ui.available_width().max(100.0);
                    let height = (outer.y - 8.0).max(40.0);
                    let request_focus = std::mem::take(&mut doc.request_focus);
                    let scroll_to = doc.scroll_to.take();
                    // The inner (sideways) scroll area swallows vertical scroll requests
                    // too, so the rect to keep in view comes out and is asked for here.
                    let keep_visible = egui::ScrollArea::horizontal()
                        .id_salt(("editor-hscroll", doc.id))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let out = egui::TextEdit::multiline(text)
                                .id(id)
                                .code_editor()
                                .font(font.clone())
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::symmetric(2, 4))
                                .lock_focus(true)
                                .desired_width(width)
                                .min_size(egui::vec2(width, height))
                                .layouter(&mut layouter)
                                .show(ui);
                            if request_focus {
                                out.response.response.request_focus();
                            }
                            let at = |ci: usize| {
                                out.galley
                                    .pos_from_cursor(CCursor::new(ci))
                                    .translate(out.galley_pos.to_vec2())
                            };
                            if let Some(b) = scroll_to {
                                let rect = at(byte_to_char(text, b)).expand(24.0);
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                return Some((rect, Some(egui::Align::Center)));
                            }
                            let cursor = out.cursor_range.map(|r| r.primary.index.0);
                            let moved = cursor != *last_cursor;
                            *last_cursor = cursor;
                            match cursor {
                                Some(ci) if moved || out.response.response.changed() => {
                                    let rect = at(ci).expand2(egui::vec2(8.0, 4.0));
                                    ui.scroll_to_rect(rect, None);
                                    Some((rect, None))
                                }
                                _ => None,
                            }
                        })
                        .inner;
                    if let Some((rect, align)) = keep_visible {
                        ui.scroll_to_rect(rect, align);
                    }
                });
            });
    });
}

/// The layout for highlighted SQL: token colours, plus backgrounds for find matches.
fn highlight(
    text: &str,
    dialect: Dialect,
    matches: &[Range<usize>],
    current: Option<usize>,
    dark: bool,
    font: &FontId,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    // Matches are found on the text before this frame's edit: ignore any that no
    // longer fit it.
    let valid = matches.iter().all(|m| {
        m.end <= text.len() && text.is_char_boundary(m.start) && text.is_char_boundary(m.end)
    });
    let matches: &[Range<usize>] = if valid { matches } else { &[] };
    let mut mi = 0;
    for t in sql::tokenize(text, dialect) {
        let color = theme::token_color(t.kind, dark);
        let mut s = t.range.start;
        while s < t.range.end {
            while mi < matches.len() && matches[mi].end <= s {
                mi += 1;
            }
            let (e, bg) = match matches.get(mi) {
                Some(m) if m.start <= s => (
                    m.end.min(t.range.end),
                    theme::match_bg(current == Some(mi), dark),
                ),
                Some(m) => (m.start.min(t.range.end), egui::Color32::TRANSPARENT),
                None => (t.range.end, egui::Color32::TRANSPARENT),
            };
            job.append(
                &text[s..e],
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    background: bg,
                    ..TextFormat::default()
                },
            );
            s = e;
        }
    }
    if text.is_empty() {
        job.append(
            "",
            0.0,
            TextFormat {
                font_id: font.clone(),
                ..TextFormat::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(text: &'a str, q: &str, case: bool) -> Vec<&'a str> {
        find_all(text, q, case)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn finding() {
        assert_eq!(
            found("Select select SELECT", "select", false),
            ["Select", "select", "SELECT"]
        );
        assert_eq!(found("Select select SELECT", "select", true), ["select"]);
        assert_eq!(found("aaaa", "aa", true), ["aa", "aa"]);
        assert_eq!(found("café CAFÉ", "café", false), ["café", "CAFÉ"]);
        assert_eq!(found("ünï", "x", false), Vec::<&str>::new());
        assert!(found("anything", "", false).is_empty());
    }

    #[test]
    fn char_byte_conversion() {
        let t = "aé😀b";
        assert_eq!(char_to_byte(t, 2), 3);
        assert_eq!(char_to_byte(t, 99), t.len());
        assert_eq!(byte_to_char(t, 7), 3);
    }

    #[test]
    fn highlight_covers_all_text() {
        let text = "select 'x' from t -- c";
        let job = highlight(
            text,
            Dialect::Postgres,
            std::slice::from_ref(&(7..10)),
            Some(0),
            true,
            &FontId::monospace(12.0),
        );
        assert_eq!(job.text, text);
        // Stale matches are ignored rather than panicking.
        let job = highlight(
            "é",
            Dialect::Postgres,
            std::slice::from_ref(&(1..2)),
            None,
            true,
            &FontId::monospace(12.0),
        );
        assert_eq!(job.text, "é");
    }
}
