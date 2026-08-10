use super::editor::EditorAction;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Map a terminal key event to a logical editor action.
/// macOS: Super (Cmd) for line/copy/undo, Alt (Option) for word nav.
/// Other platforms: Ctrl+A/E for line, Ctrl+W for word-delete, Alt for word nav.
pub fn map_key(key: KeyEvent) -> Option<EditorAction> {
    let sup = key.modifiers.contains(KeyModifiers::SUPER);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let mac = cfg!(target_os = "macos");
    let meta = mac && sup; // Cmd on mac

    match key.code {
        KeyCode::Left => Some(if alt {
            sel(
                shift,
                EditorAction::SelectWordLeft,
                EditorAction::MoveWordLeft,
            )
        } else if meta {
            sel(
                shift,
                EditorAction::SelectToLineStart,
                EditorAction::MoveToLineStart,
            )
        } else {
            sel(shift, EditorAction::SelectLeft, EditorAction::MoveLeft)
        }),
        KeyCode::Right => Some(if alt {
            sel(
                shift,
                EditorAction::SelectWordRight,
                EditorAction::MoveWordRight,
            )
        } else if meta {
            sel(
                shift,
                EditorAction::SelectToLineEnd,
                EditorAction::MoveToLineEnd,
            )
        } else {
            sel(shift, EditorAction::SelectRight, EditorAction::MoveRight)
        }),
        KeyCode::Up => Some(if meta {
            sel(
                shift,
                EditorAction::SelectToDocStart,
                EditorAction::MoveToDocStart,
            )
        } else {
            sel(shift, EditorAction::SelectUp, EditorAction::MoveUp)
        }),
        KeyCode::Down => Some(if meta {
            sel(
                shift,
                EditorAction::SelectToDocEnd,
                EditorAction::MoveToDocEnd,
            )
        } else {
            sel(shift, EditorAction::SelectDown, EditorAction::MoveDown)
        }),
        KeyCode::Home => Some(sel(
            shift,
            EditorAction::SelectToLineStart,
            EditorAction::MoveToLineStart,
        )),
        KeyCode::End => Some(sel(
            shift,
            EditorAction::SelectToLineEnd,
            EditorAction::MoveToLineEnd,
        )),
        KeyCode::Backspace => Some(if alt {
            EditorAction::DeleteWordBackward
        } else if meta {
            EditorAction::DeleteToLineStart
        } else if ctrl {
            EditorAction::DeleteWordBackward // readline Ctrl+W
        } else {
            EditorAction::DeleteBackward
        }),
        KeyCode::Delete => Some(EditorAction::DeleteForward),
        KeyCode::Char(c) => {
            if ctrl {
                return Some(match c {
                    'a' => sel(
                        shift,
                        EditorAction::SelectToLineStart,
                        EditorAction::MoveToLineStart,
                    ),
                    'e' => sel(
                        shift,
                        EditorAction::SelectToLineEnd,
                        EditorAction::MoveToLineEnd,
                    ),
                    'j' => EditorAction::InsertNewline,
                    'w' => EditorAction::DeleteWordBackward,
                    'u' => EditorAction::DeleteToLineStart,
                    'y' => EditorAction::Redo,
                    'z' if shift => EditorAction::Redo,
                    'z' => EditorAction::Undo,
                    _ => return None,
                });
            }
            if sup {
                return Some(match c {
                    'a' => EditorAction::SelectAll,
                    'c' => EditorAction::Copy,
                    'x' => EditorAction::Cut,
                    'v' => EditorAction::PasteClipboard,
                    'z' if shift => EditorAction::Redo,
                    'z' => EditorAction::Undo,
                    'y' => EditorAction::Redo,
                    _ => return None,
                });
            }
            match c {
                '\r' | '\n' => None, // Enter is handled by the caller
                _ => Some(EditorAction::InsertChar(c)),
            }
        }
        _ => None,
    }
}

fn sel(shift: bool, with: EditorAction, without: EditorAction) -> EditorAction {
    if shift {
        with
    } else {
        without
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods).kind = KeyEventKind::Press;
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_arrows_move() {
        assert_eq!(
            map_key(key(KeyCode::Left, KeyModifiers::empty())),
            Some(EditorAction::MoveLeft)
        );
        assert_eq!(
            map_key(key(KeyCode::Right, KeyModifiers::empty())),
            Some(EditorAction::MoveRight)
        );
        assert_eq!(
            map_key(key(KeyCode::Up, KeyModifiers::empty())),
            Some(EditorAction::MoveUp)
        );
    }

    #[test]
    fn shift_arrows_select() {
        assert_eq!(
            map_key(key(KeyCode::Left, KeyModifiers::SHIFT)),
            Some(EditorAction::SelectLeft)
        );
        assert_eq!(
            map_key(key(KeyCode::Up, KeyModifiers::SHIFT)),
            Some(EditorAction::SelectUp)
        );
    }

    #[test]
    fn word_nav_via_alt() {
        assert_eq!(
            map_key(key(KeyCode::Left, KeyModifiers::ALT)),
            Some(EditorAction::MoveWordLeft)
        );
        assert_eq!(
            map_key(key(KeyCode::Right, KeyModifiers::ALT)),
            Some(EditorAction::MoveWordRight)
        );
        assert_eq!(
            map_key(key(KeyCode::Backspace, KeyModifiers::ALT)),
            Some(EditorAction::DeleteWordBackward)
        );
    }

    #[test]
    fn ctrl_line_and_edit() {
        assert_eq!(
            map_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(EditorAction::MoveToLineStart)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Some(EditorAction::MoveToLineEnd)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Some(EditorAction::DeleteWordBackward)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Some(EditorAction::Undo)
        );
    }

    #[test]
    fn super_copy_paste_undo() {
        let sup = KeyModifiers::SUPER;
        assert_eq!(
            map_key(key(KeyCode::Char('c'), sup)),
            Some(EditorAction::Copy)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('v'), sup)),
            Some(EditorAction::PasteClipboard)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('a'), sup)),
            Some(EditorAction::SelectAll)
        );
        assert_eq!(
            map_key(key(KeyCode::Char('z'), sup | KeyModifiers::SHIFT)),
            Some(EditorAction::Redo)
        );
    }
}
