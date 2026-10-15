use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{num::NonZeroUsize, path::PathBuf};

pub struct SkipDialog {
    pub index: usize,
    pub input: PathBuf,
    pub vmaf: bool,
    pub value: String,
    pub finished: bool,
    replace: bool,
}

pub enum SkipAction {
    None,
    Close,
    Cancel,
    Submit(NonZeroUsize),
}

impl SkipDialog {
    pub fn new(index: usize, input: PathBuf, vmaf: bool) -> Self {
        Self {
            index,
            input,
            vmaf,
            value: "1".into(),
            finished: false,
            replace: true,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SkipAction {
        match key.code {
            KeyCode::Esc => return SkipAction::Close,
            KeyCode::Char('q') => return SkipAction::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return SkipAction::Cancel;
            }
            KeyCode::Enter if !self.finished => {
                if let Ok(count) = self.value.parse() {
                    return SkipAction::Submit(count);
                }
            }
            KeyCode::Char(value) if value.is_ascii_digit() => {
                if self.replace {
                    self.value.clear();
                }
                self.replace = false;
                self.value.push(value);
            }
            KeyCode::Backspace => {
                self.replace = false;
                self.value.pop();
            }
            KeyCode::Char('k' | 'l') | KeyCode::Up | KeyCode::Right => {
                self.value = self
                    .value
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_add(1)
                    .to_string();
                self.replace = true;
            }
            KeyCode::Char('j' | 'h') | KeyCode::Down | KeyCode::Left => {
                self.value = self
                    .value
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .max(1)
                    .to_string();
                self.replace = true;
            }
            _ => {}
        }
        SkipAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_up_increases_and_vim_down_decreases_the_count() {
        let mut dialog = SkipDialog::new(1, "current.mkv".into(), false);
        dialog.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(dialog.value, "2");
        dialog.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(dialog.value, "1");
    }
}
