use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    Restart,
    ToggleFilter,
    ClearFilter,
    ScrollUp,
    ScrollDown,
    PrevService,
    NextService,
    None,
}

pub fn handle_key(key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('r') => Action::Restart,
        KeyCode::Char('f') | KeyCode::Enter => Action::ToggleFilter,
        KeyCode::Esc => Action::ClearFilter,
        // Shift+Up/Down or PgUp/PgDn scroll logs
        KeyCode::Up if shift => Action::ScrollUp,
        KeyCode::Down if shift => Action::ScrollDown,
        KeyCode::PageUp => Action::ScrollUp,
        KeyCode::PageDown => Action::ScrollDown,
        // Up/Down navigate services
        KeyCode::Up => Action::PrevService,
        KeyCode::Down => Action::NextService,
        KeyCode::Tab => Action::NextService,
        KeyCode::BackTab => Action::PrevService,
        _ => Action::None,
    }
}
