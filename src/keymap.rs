#![allow(dead_code)] // Wired into the live session loop in Plan-2 Task 11.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Finish,       // space — save & close (release, in ephemeral)
    ToggleRaw,    // u
    TogglePause,  // p
    Cancel,       // esc / ctrl-c — discard (the live loop then shows a y/n confirm)
    None,
}

pub fn action_for(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char(' ') => Action::Finish,
        KeyCode::Char('u') => Action::ToggleRaw,
        KeyCode::Char('p') => Action::TogglePause,
        KeyCode::Esc => Action::Cancel,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Cancel,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

    #[test]
    fn maps_the_core_keys() {
        assert_eq!(action_for(k(KeyCode::Char(' '))), Action::Finish);
        assert_eq!(action_for(k(KeyCode::Char('u'))), Action::ToggleRaw);
        assert_eq!(action_for(k(KeyCode::Char('p'))), Action::TogglePause);
        assert_eq!(action_for(k(KeyCode::Esc)), Action::Cancel);
        assert_eq!(action_for(k(KeyCode::Char('x'))), Action::None);
    }

    #[test]
    fn ctrl_c_cancels() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_c), Action::Cancel);
        assert_eq!(action_for(k(KeyCode::Char('c'))), Action::None);
    }
}
