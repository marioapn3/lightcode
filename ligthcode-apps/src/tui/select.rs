use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A selection point in content-area coordinates `(row, col)`.
pub type SelPoint = (usize, usize);
/// An unordered mouse selection `(anchor, focus)`.
pub type Selection = (SelPoint, SelPoint);

/// Display width of a string in terminal columns.
pub fn line_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Map a display column to a char index in `text`.
pub fn col_to_char(text: &str, col: usize) -> usize {
    let mut w = 0usize;
    for (i, c) in text.char_indices() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1);
        if w + cw > col {
            return i;
        }
        w += cw;
    }
    text.len()
}

/// Slice `text` to the display-column range `[from, to)`.
pub fn slice_by_cols(text: &str, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1);
        if w >= to {
            break;
        }
        if w + cw > from {
            out.push(c);
        }
        w += cw;
    }
    out
}

/// Simulate terminal wrapping of the built content: one entry per terminal row,
/// `(line_idx, col_from, col_to)` into that line's display columns.
pub fn wrap_rows(lines: &[Line], width: usize) -> Vec<(usize, usize, usize)> {
    let width = width.max(1);
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let w = line_width(&text);
        if w == 0 {
            out.push((li, 0, 0));
            continue;
        }
        let rows = w.div_ceil(width);
        for k in 0..rows {
            let cf = k * width;
            let ct = (cf + width).min(w);
            out.push((li, cf, ct));
        }
    }
    out
}

/// Normalize an unordered (anchor, focus) pair into (start, end).
pub fn sel_normalized(a: SelPoint, b: SelPoint) -> (SelPoint, SelPoint) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Render the visible content rows with the mouse selection highlighted.
/// `sel` is in content-area coordinates `(row, col)`; `scroll` is the content
/// scroll offset used when the frame was drawn.
pub fn visible_rows(
    lines: &[Line],
    width: usize,
    scroll: usize,
    height: usize,
    sel: Option<&Selection>,
) -> Vec<Line<'static>> {
    let rows = wrap_rows(lines, width);
    let total = rows.len();
    let start = scroll.min(total.saturating_sub(height.max(1)));
    let end = (start + height.max(1)).min(total);
    let mut out = Vec::with_capacity(end - start);
    for (r, (li, cf, ct)) in rows[start..end].iter().enumerate() {
        let abs_row = start + r;
        let text: String = lines[*li]
            .spans
            .iter()
            .map(|sp| sp.content.as_ref())
            .collect();
        let seg = slice_by_cols(&text, *cf, *ct);
        if let Some(sel) = sel {
            let (s, e) = sel_normalized(sel.0, sel.1);
            let s_row = s.0 + scroll;
            let e_row = e.0 + scroll;
            if abs_row >= s_row && abs_row <= e_row {
                let seg_w = ct.saturating_sub(*cf);
                let from = if abs_row == s_row { s.1.min(seg_w) } else { 0 };
                let to = if abs_row == e_row {
                    e.1.min(seg_w)
                } else {
                    seg_w
                };
                let from = from.min(to);
                out.push(highlight(&seg, from, to));
                continue;
            }
        }
        out.push(Line::raw(seg));
    }
    out
}

fn highlight(seg: &str, from: usize, to: usize) -> Line<'static> {
    let a = col_to_char(seg, from);
    let b = col_to_char(seg, to);
    Line::from(vec![
        Span::raw(seg[..a].to_string()),
        Span::styled(
            seg[a..b].to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ),
        Span::raw(seg[b..].to_string()),
    ])
}

/// Extract the mouse-selected text from the rendered content.
pub fn extract_text(lines: &[Line], width: usize, scroll: usize, sel: &Selection) -> String {
    let (s, e) = sel_normalized(sel.0, sel.1);
    let s_row = s.0 + scroll;
    let e_row = e.0 + scroll;
    let rows = wrap_rows(lines, width);
    let mut out = String::new();
    for (r, (li, cf, ct)) in rows.iter().enumerate() {
        if r < s_row || r > e_row {
            continue;
        }
        let text: String = lines[*li]
            .spans
            .iter()
            .map(|sp| sp.content.as_ref())
            .collect();
        let seg = slice_by_cols(&text, *cf, *ct);
        let seg_w = ct.saturating_sub(*cf);
        if r == s_row && r == e_row {
            let a = col_to_char(&seg, s.1.min(seg_w));
            let b = col_to_char(&seg, e.1.min(seg_w));
            if a <= b {
                out.push_str(&seg[a..b]);
            }
        } else if r == s_row {
            out.push_str(&seg[col_to_char(&seg, s.1.min(seg_w))..]);
            out.push('\n');
        } else if r == e_row {
            out.push_str(&seg[..col_to_char(&seg, e.1.min(seg_w))]);
        } else {
            out.push_str(&seg);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<Line<'static>> {
        texts.iter().map(|t| Line::raw(t.to_string())).collect()
    }

    #[test]
    fn col_mapping_and_slicing() {
        assert_eq!(col_to_char("hello", 3), 3);
        // "a🙂b": col 3 is just past 🙂 (2 wide) → byte index 5 ('b').
        assert_eq!(col_to_char("a🙂b", 3), 5);
        assert_eq!(slice_by_cols("hello world", 0, 5), "hello");
    }

    #[test]
    fn wrap_rows_handles_long_lines() {
        let l = lines(&["abcdefgh", "x"]);
        let rows = wrap_rows(&l, 4);
        assert_eq!(rows.len(), 3); // 8/4 = 2 + 1
        assert_eq!(rows[0], (0, 0, 4));
        assert_eq!(rows[1], (0, 4, 8));
        assert_eq!(rows[2], (1, 0, 1));
    }

    #[test]
    fn extract_single_row_selection() {
        let l = lines(&["hello world"]);
        let sel = ((0, 0), (0, 5));
        assert_eq!(extract_text(&l, 80, 0, &sel), "hello");
    }

    #[test]
    fn extract_multi_row_selection() {
        let l = lines(&["abcd", "efgh"]);
        let sel = ((0, 1), (1, 2));
        assert_eq!(extract_text(&l, 80, 0, &sel), "bcd\nef");
    }

    #[test]
    fn selection_reversed_regardless_of_direction() {
        let sel = ((2, 5), (0, 1));
        let (s, e) = sel_normalized(sel.0, sel.1);
        assert_eq!(s, (0, 1));
        assert_eq!(e, (2, 5));
    }
}
