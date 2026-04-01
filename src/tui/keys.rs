use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    Restart,
    Filter,
    ClearFilter,
    ScrollUp,
    ScrollDown,
    NextService,
    None,
}

pub fn handle_key(key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('r') => Action::Restart,
        KeyCode::Char('f') => Action::Filter,
        KeyCode::Esc => Action::ClearFilter,
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        KeyCode::Tab => Action::NextService,
        _ => Action::None,
    }
}
