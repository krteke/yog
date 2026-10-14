use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_runtime::recovery::{RecoveryAction, RecoveryRequest};

pub struct RecoveryDialog {
    pub request: RecoveryRequest,
    pub selected: usize,
    pub apply_to_similar: bool,
    pub visible: bool,
    pub scroll: u16,
}

pub enum RecoveryDialogAction {
    None,
    Close,
    Cancel,
    Respond,
}

impl RecoveryDialog {
    pub fn new(request: RecoveryRequest) -> Self {
        Self {
            request,
            selected: 0,
            apply_to_similar: false,
            visible: true,
            scroll: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> RecoveryDialogAction {
        match key.code {
            KeyCode::Char('q') => return RecoveryDialogAction::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return RecoveryDialogAction::Cancel;
            }
            KeyCode::Esc => return RecoveryDialogAction::Close,
            KeyCode::Enter => return RecoveryDialogAction::Respond,
            KeyCode::Char('j' | 'l') | KeyCode::Down | KeyCode::Right => {
                self.selected = (self.selected + 1) % RecoveryAction::ALL.len()
            }
            KeyCode::Char('k' | 'h') | KeyCode::Up | KeyCode::Left => {
                self.selected =
                    (self.selected + RecoveryAction::ALL.len() - 1) % RecoveryAction::ALL.len()
            }
            KeyCode::Char(' ') => self.apply_to_similar = !self.apply_to_similar,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(3),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(3),
            KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
            _ => {}
        }
        RecoveryDialogAction::None
    }

    pub fn respond(self) {
        self.request
            .respond(RecoveryAction::ALL[self.selected], self.apply_to_similar);
    }
}
