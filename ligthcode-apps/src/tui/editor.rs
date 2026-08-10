use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// One line of text, stored as a vector of grapheme clusters.
type Line = Vec<String>;

fn to_graphemes(s: &str) -> Line {
    s.graphemes(true).map(|g| g.to_string()).collect()
}

fn from_graphemes(v: &[String]) -> String {
    v.concat()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

impl PartialOrd for Cursor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cursor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.row, self.col).cmp(&(other.row, other.col))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cursor,
    pub focus: Cursor,
}

impl Selection {
    fn range(&self) -> (Cursor, Cursor) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
}

#[derive(Debug, Clone)]
struct Edit {
    lines: Vec<Line>,
    cursor: Cursor,
    selection: Option<Selection>,
}

/// Logical editor actions, mapped from terminal-specific key events elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveToLineStart,
    MoveToLineEnd,
    MoveToDocStart,
    MoveToDocEnd,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectToLineStart,
    SelectToLineEnd,
    SelectToDocStart,
    SelectToDocEnd,
    SelectAll,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteToLineStart,
    InsertChar(char),
    InsertNewline,
    Copy,
    Cut,
    /// Paste the editor's internal clipboard.
    PasteClipboard,
    /// Insert terminal paste content (may contain newlines) as a single logical op.
    Paste(String),
    Undo,
    Redo,
}

/// A small multiline text editor, independent of the LLM/agent.
pub struct TextEditor {
    lines: Vec<Line>,
    cursor: Cursor,
    selection: Option<Selection>,
    scroll_row: usize,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    clipboard: String,
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            cursor: Cursor { row: 0, col: 0 },
            selection: None,
            scroll_row: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: String::new(),
        }
    }

    pub fn load_text(&mut self, text: &str) {
        let mut lines: Vec<Line> = text.split('\n').map(to_graphemes).collect();
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        self.lines = lines;
        self.cursor = Cursor { row: 0, col: 0 };
        self.selection = None;
        self.undo.clear();
        self.redo.clear();
        self.scroll_row = 0;
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| from_graphemes(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    pub fn clear(&mut self) {
        self.load_text("");
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn scroll_row(&self) -> usize {
        self.scroll_row
    }

    /// Ensure the cursor line is within the visible window of `max_visible` rows.
    pub fn scroll_to_cursor(&mut self, max_visible: usize) {
        let max_visible = max_visible.max(1);
        if self.cursor.row < self.scroll_row {
            self.scroll_row = self.cursor.row;
        } else if self.cursor.row >= self.scroll_row + max_visible {
            self.scroll_row = self.cursor.row + 1 - max_visible;
        }
    }

    /// Display-width (in terminal columns) of the current line up to the cursor.
    pub fn cursor_col_width(&self) -> usize {
        let line = &self.lines[self.cursor.row];
        line[..self.cursor.col.min(line.len())]
            .iter()
            .map(|g| UnicodeWidthStr::width(g.as_str()))
            .sum()
    }

    /// Graphemes of a line, for rendering.
    pub fn line_graphemes(&self, row: usize) -> Vec<String> {
        self.lines.get(row).cloned().unwrap_or_default()
    }

    /// Selection range on a specific row (col_start, col_end) in graphemes, if any.
    pub fn selection_for_row(&self, row: usize) -> Option<(usize, usize)> {
        let sel = self.selection?;
        let (start, end) = sel.range();
        if row < start.row || row > end.row {
            return None;
        }
        let s = if row == start.row { start.col } else { 0 };
        let e = if row == end.row {
            end.col
        } else {
            self.lines[row].len()
        };
        Some((s, e))
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection?.range();
        let mut out = String::new();
        for r in start.row..=end.row {
            let line = &self.lines[r];
            let s = if r == start.row { start.col } else { 0 };
            let e = if r == end.row { end.col } else { line.len() };
            if r > start.row {
                out.push('\n');
            }
            out.push_str(&from_graphemes(&line[s.min(line.len())..e.min(line.len())]));
        }
        Some(out)
    }

    pub fn clipboard_is_empty(&self) -> bool {
        self.clipboard.is_empty()
    }

    /// Insert text (may contain newlines) at the cursor as a single edit step.
    /// Used for terminal paste.
    pub fn insert_plain(&mut self, text: &str) {
        self.begin_edit();
        if let Some(sel) = self.selection.take() {
            let (s, e) = sel.range();
            self.delete_range(s, e);
        }
        self.insert_text(text);
    }

    /// Replace the current line (at the cursor) with `replacement`, which may
    /// contain newlines. Single undo step. Used to expand paste placeholders.
    pub fn replace_current_line(&mut self, replacement: &str) {
        self.begin_edit();
        let row = self.cursor.row;
        let len = self.lines[row].len();
        self.delete_range(Cursor { row, col: 0 }, Cursor { row, col: len });
        self.insert_text(replacement);
    }

    /// Byte offset of the cursor in the editor's text.
    pub fn cursor_byte_offset(&self) -> usize {
        let mut off = 0usize;
        for (r, line) in self.lines.iter().enumerate() {
            if r == self.cursor.row {
                for g in line.iter().take(self.cursor.col.min(line.len())) {
                    off += g.len();
                }
                return off;
            }
            off += from_graphemes(line).len() + 1; // + newline
        }
        off
    }

    /// Convert a byte offset into the editor's text to a (row, col) cursor.
    pub fn cursor_from_byte_offset(&self, byte: usize) -> Cursor {
        let mut off = 0usize;
        for (r, line) in self.lines.iter().enumerate() {
            let l = from_graphemes(line);
            if off + l.len() >= byte || r == self.lines.len() - 1 {
                let local = byte.saturating_sub(off).min(l.len());
                let mut col = 0usize;
                let mut used = 0usize;
                for g in line {
                    if used + g.len() > local {
                        break;
                    }
                    used += g.len();
                    col += 1;
                }
                return Cursor { row: r, col };
            }
            off += l.len() + 1;
        }
        Cursor { row: 0, col: 0 }
    }

    /// Replace the byte range `[start, end)` with `replacement` as a single
    /// undo step, leaving the cursor after the inserted text.
    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        self.begin_edit();
        let s = self.cursor_from_byte_offset(start);
        let e = self.cursor_from_byte_offset(end);
        self.delete_range(s, e);
        self.insert_text(replacement);
    }

    pub fn apply(&mut self, action: EditorAction) {
        match action {
            EditorAction::MoveLeft => self.move_h(-1, false),
            EditorAction::MoveRight => self.move_h(1, false),
            EditorAction::MoveUp => self.move_v(-1, false),
            EditorAction::MoveDown => self.move_v(1, false),
            EditorAction::MoveWordLeft => self.move_word(-1, false),
            EditorAction::MoveWordRight => self.move_word(1, false),
            EditorAction::MoveToLineStart => self.move_line_start(false),
            EditorAction::MoveToLineEnd => self.move_line_end(false),
            EditorAction::MoveToDocStart => {
                self.begin_edit();
                self.cursor = Cursor { row: 0, col: 0 };
                self.selection = None;
            }
            EditorAction::MoveToDocEnd => {
                self.begin_edit();
                let last = self.lines.len() - 1;
                self.cursor = Cursor {
                    row: last,
                    col: self.lines[last].len(),
                };
                self.selection = None;
            }
            EditorAction::SelectLeft => self.move_h(-1, true),
            EditorAction::SelectRight => self.move_h(1, true),
            EditorAction::SelectUp => self.move_v(-1, true),
            EditorAction::SelectDown => self.move_v(1, true),
            EditorAction::SelectWordLeft => self.move_word(-1, true),
            EditorAction::SelectWordRight => self.move_word(1, true),
            EditorAction::SelectToLineStart => self.move_line_start(true),
            EditorAction::SelectToLineEnd => self.move_line_end(true),
            EditorAction::SelectToDocStart => self.move_to_doc(true),
            EditorAction::SelectToDocEnd => self.move_to_doc(false),
            EditorAction::SelectAll => {
                self.begin_edit();
                let last = self.lines.len() - 1;
                self.selection = Some(Selection {
                    anchor: Cursor { row: 0, col: 0 },
                    focus: Cursor {
                        row: last,
                        col: self.lines[last].len(),
                    },
                });
                self.cursor = self.selection.unwrap().focus;
            }
            EditorAction::DeleteBackward => self.delete_backward(),
            EditorAction::DeleteForward => self.delete_forward(),
            EditorAction::DeleteWordBackward => self.delete_word_backward(),
            EditorAction::DeleteToLineStart => self.delete_to_line_start(),
            EditorAction::InsertChar(c) => self.insert_char(c),
            EditorAction::InsertNewline => self.insert_newline(),
            EditorAction::Copy => {
                if let Some(text) = self.selected_text() {
                    self.clipboard = text;
                }
            }
            EditorAction::Cut => {
                if self.selection.is_some() {
                    self.begin_edit();
                    if let Some(text) = self.selected_text() {
                        self.clipboard = text;
                    }
                    let (s, e) = self.selection.take().unwrap().range();
                    self.delete_range(s, e);
                }
            }
            EditorAction::PasteClipboard => {
                let text = self.clipboard.clone();
                if !text.is_empty() {
                    self.insert_plain(&text);
                }
            }
            EditorAction::Paste(text) => self.insert_plain(&text),
            EditorAction::Undo => self.undo(),
            EditorAction::Redo => self.redo(),
        }
    }

    // --- editing internals ---

    fn begin_edit(&mut self) {
        self.undo.push(Edit {
            lines: self.lines.clone(),
            cursor: self.cursor,
            selection: self.selection,
        });
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn insert_text(&mut self, text: &str) {
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or("");
        for g in to_graphemes(first) {
            self.lines[self.cursor.row].insert(self.cursor.col, g);
            self.cursor.col += 1;
        }
        for part in parts {
            self.insert_newline_impl();
            for g in to_graphemes(part) {
                self.lines[self.cursor.row].insert(self.cursor.col, g);
                self.cursor.col += 1;
            }
        }
    }

    fn insert_char(&mut self, c: char) {
        self.begin_edit();
        if let Some(sel) = self.selection.take() {
            let (s, e) = sel.range();
            self.delete_range(s, e);
        }
        if c == '\n' {
            self.insert_newline_impl();
        } else {
            self.lines[self.cursor.row].insert(self.cursor.col, c.to_string());
            self.cursor.col += 1;
        }
    }

    fn insert_newline(&mut self) {
        self.begin_edit();
        if let Some(sel) = self.selection.take() {
            let (s, e) = sel.range();
            self.delete_range(s, e);
        }
        self.insert_newline_impl();
    }

    fn insert_newline_impl(&mut self) {
        let line = &self.lines[self.cursor.row];
        let right = line[self.cursor.col..].to_vec();
        self.lines[self.cursor.row].truncate(self.cursor.col);
        self.lines.insert(self.cursor.row + 1, right);
        self.cursor.row += 1;
        self.cursor.col = 0;
    }

    fn delete_backward(&mut self) {
        if let Some(sel) = self.selection {
            self.begin_edit();
            let (s, e) = sel.range();
            self.selection = None;
            self.delete_range(s, e);
            return;
        }
        self.begin_edit();
        if self.cursor.col > 0 {
            self.lines[self.cursor.row].remove(self.cursor.col - 1);
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            let prev_len = self.lines[self.cursor.row - 1].len();
            let cur = self.lines.remove(self.cursor.row);
            self.lines[self.cursor.row - 1].extend(cur);
            self.cursor.row -= 1;
            self.cursor.col = prev_len;
        }
    }

    fn delete_forward(&mut self) {
        if let Some(sel) = self.selection {
            self.begin_edit();
            let (s, e) = sel.range();
            self.selection = None;
            self.delete_range(s, e);
            return;
        }
        self.begin_edit();
        let line = &mut self.lines[self.cursor.row];
        if self.cursor.col < line.len() {
            line.remove(self.cursor.col);
        } else if self.cursor.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor.row + 1);
            self.lines[self.cursor.row].extend(next);
        }
    }

    fn delete_word_backward(&mut self) {
        if self.selection.is_some() {
            self.delete_backward();
            return;
        }
        self.begin_edit();
        let start = self.word_left_pos();
        self.delete_range(start, self.cursor);
    }

    fn delete_to_line_start(&mut self) {
        if self.selection.is_some() {
            self.delete_backward();
            return;
        }
        self.begin_edit();
        let start = Cursor {
            row: self.cursor.row,
            col: 0,
        };
        self.delete_range(start, self.cursor);
    }

    fn delete_range(&mut self, start: Cursor, end: Cursor) {
        if start == end {
            return;
        }
        if start.row == end.row {
            let line = &mut self.lines[start.row];
            line.drain(start.col.min(line.len())..end.col.min(line.len()));
        } else {
            let mut merged =
                self.lines[start.row][..start.col.min(self.lines[start.row].len())].to_vec();
            merged
                .extend_from_slice(&self.lines[end.row][end.col.min(self.lines[end.row].len())..]);
            let mut new_lines = self.lines[..start.row].to_vec();
            new_lines.push(merged);
            new_lines.extend(self.lines[end.row + 1..].to_vec());
            self.lines = new_lines;
        }
        self.cursor = start;
    }

    // --- movement internals ---

    fn move_h(&mut self, delta: isize, extend: bool) {
        if !extend && self.selection.is_some() {
            let (s, _) = self.selection.take().unwrap().range();
            self.cursor = s;
            return;
        }
        let prev = self.cursor;
        if delta < 0 {
            if self.cursor.col > 0 {
                self.cursor.col -= 1;
            } else if self.cursor.row > 0 {
                self.cursor.row -= 1;
                self.cursor.col = self.lines[self.cursor.row].len();
            }
        } else {
            let line = &self.lines[self.cursor.row];
            if self.cursor.col < line.len() {
                self.cursor.col += 1;
            } else if self.cursor.row + 1 < self.lines.len() {
                self.cursor.row += 1;
                self.cursor.col = 0;
            }
        }
        self.update_selection(prev, extend);
    }

    fn move_v(&mut self, delta: isize, extend: bool) {
        if !extend && self.selection.is_some() {
            let (s, _) = self.selection.take().unwrap().range();
            self.cursor = s;
            return;
        }
        let prev = self.cursor;
        let col = self.cursor.col;
        if delta < 0 {
            if self.cursor.row == 0 {
                self.cursor.col = 0;
            } else {
                self.cursor.row -= 1;
                self.cursor.col = col.min(self.lines[self.cursor.row].len());
            }
        } else if self.cursor.row + 1 >= self.lines.len() {
            self.cursor.col = self.lines[self.cursor.row].len();
        } else {
            self.cursor.row += 1;
            self.cursor.col = col.min(self.lines[self.cursor.row].len());
        }
        self.update_selection(prev, extend);
    }

    fn move_word(&mut self, dir: isize, extend: bool) {
        if !extend && self.selection.is_some() {
            let (s, _) = self.selection.take().unwrap().range();
            self.cursor = s;
            return;
        }
        let prev = self.cursor;
        self.cursor = if dir < 0 {
            self.word_left_pos()
        } else {
            self.word_right_pos()
        };
        self.update_selection(prev, extend);
    }

    fn move_line_start(&mut self, extend: bool) {
        if !extend && self.selection.is_some() {
            let (s, _) = self.selection.take().unwrap().range();
            self.cursor = s;
            return;
        }
        let prev = self.cursor;
        self.cursor.col = 0;
        self.update_selection(prev, extend);
    }

    fn move_line_end(&mut self, extend: bool) {
        if !extend && self.selection.is_some() {
            let (s, _) = self.selection.take().unwrap().range();
            self.cursor = s;
            return;
        }
        let prev = self.cursor;
        self.cursor.col = self.lines[self.cursor.row].len();
        self.update_selection(prev, extend);
    }

    fn move_to_doc(&mut self, start: bool) {
        let prev = self.cursor;
        let last = self.lines.len() - 1;
        self.cursor = if start {
            Cursor { row: 0, col: 0 }
        } else {
            Cursor {
                row: last,
                col: self.lines[last].len(),
            }
        };
        self.update_selection(prev, true);
    }

    fn update_selection(&mut self, prev: Cursor, extend: bool) {
        if extend {
            self.selection = Some(match self.selection {
                Some(sel) => Selection {
                    anchor: sel.anchor,
                    focus: self.cursor,
                },
                None => Selection {
                    anchor: prev,
                    focus: self.cursor,
                },
            });
        } else {
            self.selection = None;
        }
    }

    fn word_left_pos(&self) -> Cursor {
        let (mut row, mut col) = (self.cursor.row, self.cursor.col);
        if col == 0 {
            if row == 0 {
                return Cursor { row: 0, col: 0 };
            }
            row -= 1;
            col = self.lines[row].len();
        }
        // skip whitespace
        while {
            let g = self.grapheme_before(row, col);
            g.is_some_and(|g| g.trim().is_empty())
        } {
            if let Some((r, c)) = self.step_left(row, col) {
                row = r;
                col = c;
            } else {
                break;
            }
        }
        // skip word chars
        while {
            let g = self.grapheme_before(row, col);
            g.is_some_and(|g| !g.trim().is_empty())
        } {
            if let Some((r, c)) = self.step_left(row, col) {
                row = r;
                col = c;
            } else {
                break;
            }
        }
        Cursor { row, col }
    }

    fn word_right_pos(&self) -> Cursor {
        let (mut row, mut col) = (self.cursor.row, self.cursor.col);
        if col == self.lines[row].len() {
            if row + 1 >= self.lines.len() {
                return Cursor {
                    row,
                    col: self.lines[row].len(),
                };
            }
            row += 1;
            col = 0;
        }
        // skip word chars
        while {
            let g = self.grapheme_at(row, col);
            g.is_some_and(|g| !g.trim().is_empty())
        } {
            if let Some((r, c)) = self.step_right(row, col) {
                row = r;
                col = c;
            } else {
                break;
            }
        }
        // skip whitespace
        while {
            let g = self.grapheme_at(row, col);
            g.is_some_and(|g| g.trim().is_empty())
        } {
            if let Some((r, c)) = self.step_right(row, col) {
                row = r;
                col = c;
            } else {
                break;
            }
        }
        Cursor { row, col }
    }

    fn grapheme_at(&self, row: usize, col: usize) -> Option<&str> {
        self.lines.get(row)?.get(col).map(|s| s.as_str())
    }

    fn grapheme_before(&self, row: usize, col: usize) -> Option<&str> {
        if col > 0 {
            self.lines.get(row)?.get(col - 1).map(|s| s.as_str())
        } else if row > 0 {
            self.lines.get(row - 1)?.last().map(|s| s.as_str())
        } else {
            None
        }
    }

    fn step_left(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        if col > 0 {
            Some((row, col - 1))
        } else if row > 0 {
            Some((row - 1, self.lines[row - 1].len()))
        } else {
            None
        }
    }

    fn step_right(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        if col < self.lines[row].len() {
            Some((row, col + 1))
        } else if row + 1 < self.lines.len() {
            Some((row + 1, 0))
        } else {
            None
        }
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(Edit {
                lines: self.lines.clone(),
                cursor: self.cursor,
                selection: self.selection,
            });
            self.lines = prev.lines;
            self.cursor = prev.cursor;
            self.selection = prev.selection;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(Edit {
                lines: self.lines.clone(),
                cursor: self.cursor,
                selection: self.selection,
            });
            self.lines = next.lines;
            self.cursor = next.cursor;
            self.selection = next.selection;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(ed: &mut TextEditor, s: &str) {
        for c in s.chars() {
            ed.apply(EditorAction::InsertChar(c));
        }
    }

    fn from_text_(t: &str) -> TextEditor {
        let mut ed = TextEditor::new();
        ed.load_text(t);
        ed
    }

    #[test]
    fn insert_and_text() {
        let mut ed = TextEditor::new();
        type_text(&mut ed, "hello");
        assert_eq!(ed.text(), "hello");
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 5 });
    }

    #[test]
    fn backspace_and_delete() {
        let mut ed = from_text_("hello");
        ed.apply(EditorAction::MoveToLineEnd);
        ed.apply(EditorAction::DeleteBackward);
        assert_eq!(ed.text(), "hell");
        ed.apply(EditorAction::DeleteForward);
        assert_eq!(ed.text(), "hell"); // at end, no-op
        ed.apply(EditorAction::MoveToLineStart);
        ed.apply(EditorAction::DeleteForward);
        assert_eq!(ed.text(), "ell");
    }

    #[test]
    fn cursor_navigation_multiline() {
        let mut ed = from_text_("ab\ncd");
        ed.apply(EditorAction::MoveToDocEnd);
        assert_eq!(ed.cursor(), Cursor { row: 1, col: 2 });
        ed.apply(EditorAction::MoveLeft);
        assert_eq!(ed.cursor(), Cursor { row: 1, col: 1 });
        ed.apply(EditorAction::MoveLeft);
        ed.apply(EditorAction::MoveLeft);
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 2 }); // wraps to prev line end
        ed.apply(EditorAction::MoveUp); // top line → line start
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 0 });
        ed.apply(EditorAction::MoveDown);
        assert_eq!(ed.cursor(), Cursor { row: 1, col: 0 });
        ed.apply(EditorAction::MoveDown); // at last line → to end
        assert_eq!(ed.cursor(), Cursor { row: 1, col: 2 });
    }

    #[test]
    fn newline_splits_line() {
        let mut ed = TextEditor::new();
        type_text(&mut ed, "hello world");
        ed.apply(EditorAction::MoveWordLeft);
        let pos = ed.cursor();
        ed.apply(EditorAction::InsertNewline);
        assert_eq!(ed.line_count(), 2);
        assert_eq!(ed.text(), "hello \nworld");
        assert_eq!(
            ed.cursor(),
            Cursor {
                row: pos.row + 1,
                col: 0
            }
        );
    }

    #[test]
    fn word_movement() {
        let mut ed = from_text_("hello world authentication service");
        ed.apply(EditorAction::MoveToDocEnd);
        ed.apply(EditorAction::MoveWordLeft);
        assert_eq!(ed.cursor().col, 27); // before "service"
        ed.apply(EditorAction::MoveWordLeft);
        assert_eq!(ed.cursor().col, 12); // before "authentication"
        ed.apply(EditorAction::MoveToLineStart);
        ed.apply(EditorAction::MoveWordRight);
        assert_eq!(ed.cursor().col, 6); // start of "world"
    }

    #[test]
    fn delete_word_backward() {
        let mut ed = from_text_("hello world authentication");
        ed.apply(EditorAction::MoveToDocEnd);
        ed.apply(EditorAction::DeleteWordBackward);
        assert_eq!(ed.text(), "hello world ");
        ed.apply(EditorAction::DeleteWordBackward);
        assert_eq!(ed.text(), "hello ");
    }

    #[test]
    fn delete_to_line_start() {
        let mut ed = from_text_("hello world authentication");
        ed.apply(EditorAction::MoveToDocEnd);
        ed.apply(EditorAction::DeleteToLineStart);
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn selection_delete_and_replace() {
        let mut ed = from_text_("hello world");
        // select "world" via shift+word-right from after space
        ed.apply(EditorAction::MoveToLineEnd);
        ed.apply(EditorAction::SelectWordLeft);
        assert_eq!(ed.selected_text().unwrap(), "world");
        ed.apply(EditorAction::DeleteBackward);
        assert_eq!(ed.text(), "hello ");
        // type replaces selection
        let mut ed = from_text_("hello world");
        ed.apply(EditorAction::MoveToLineEnd);
        ed.apply(EditorAction::SelectWordLeft);
        ed.apply(EditorAction::InsertChar('X'));
        assert_eq!(ed.text(), "hello X");
    }

    #[test]
    fn multiline_selection_delete() {
        let mut ed = from_text_("ab\ncd\nef");
        ed.apply(EditorAction::SelectAll);
        assert_eq!(ed.selected_text().unwrap(), "ab\ncd\nef");
        ed.apply(EditorAction::DeleteBackward);
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn undo_redo() {
        let mut ed = TextEditor::new();
        type_text(&mut ed, "hello");
        ed.apply(EditorAction::Undo);
        assert_eq!(ed.text(), "hell"); // one edit step undone
        ed.apply(EditorAction::Undo);
        assert_eq!(ed.text(), "hel");
        ed.apply(EditorAction::Redo);
        assert_eq!(ed.text(), "hell");
        ed.apply(EditorAction::Redo);
        assert_eq!(ed.text(), "hello");
    }

    #[test]
    fn paste_preserves_newlines() {
        let mut ed = TextEditor::new();
        ed.apply(EditorAction::Paste("line1\nline2".to_string()));
        assert_eq!(ed.text(), "line1\nline2");
        assert_eq!(ed.cursor(), Cursor { row: 1, col: 5 });
    }

    #[test]
    fn paste_replaces_selection_as_one_step() {
        let mut ed = from_text_("hello world");
        ed.apply(EditorAction::MoveToLineEnd);
        ed.apply(EditorAction::SelectWordLeft);
        ed.apply(EditorAction::Paste("X".to_string()));
        assert_eq!(ed.text(), "hello X");
        // single undo removes the whole paste, not one char at a time
        ed.apply(EditorAction::Undo);
        assert_eq!(ed.text(), "hello world");
    }

    #[test]
    fn copy_paste_internal_clipboard() {
        let mut ed = from_text_("hello world");
        ed.apply(EditorAction::SelectAll);
        ed.apply(EditorAction::Copy);
        ed.apply(EditorAction::DeleteBackward);
        assert_eq!(ed.text(), "");
        ed.apply(EditorAction::PasteClipboard);
        assert_eq!(ed.text(), "hello world");
    }

    #[test]
    fn emoji_grapheme_cursor() {
        let mut ed = from_text_("a👨‍👩‍👧b");
        ed.apply(EditorAction::MoveToLineEnd);
        assert_eq!(ed.cursor().col, 3); // a, family-emoji, b
        ed.apply(EditorAction::MoveLeft);
        assert_eq!(ed.cursor().col, 2);
        ed.apply(EditorAction::DeleteBackward); // deletes the family emoji as one cluster
        assert_eq!(ed.text(), "ab");
    }

    #[test]
    fn wide_char_display_width() {
        let mut ed = from_text_("a🙂b");
        ed.apply(EditorAction::MoveToLineEnd);
        // cols: a(0) 🙂(1) b(2); display width: 1 + 2 + 1 = 4
        assert_eq!(ed.cursor_col_width(), 4);
    }

    #[test]
    fn doc_boundaries() {
        let mut ed = from_text_("hi");
        ed.apply(EditorAction::MoveToDocStart);
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 0 });
        ed.apply(EditorAction::MoveLeft);
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 0 }); // stays
        ed.apply(EditorAction::MoveUp);
        assert_eq!(ed.cursor(), Cursor { row: 0, col: 0 });
    }

    #[test]
    fn history_like_load() {
        let mut ed = from_text_("first");
        assert_eq!(ed.text(), "first");
        ed.load_text("second");
        assert_eq!(ed.text(), "second");
        ed.clear();
        assert!(ed.is_empty());
    }
}
