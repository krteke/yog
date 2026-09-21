use crossterm::event::KeyEvent;

use crate::{
    file_picker::{FilePicker, PickerAction},
    form::{FormAction, TranscodeForm},
};

#[derive(Default)]
pub(crate) struct App {
    form: TranscodeForm,
    picker: Option<FilePicker>,
    error: Option<String>,
}

impl App {
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> bool {
        if let Some(picker) = &mut self.picker {
            match picker.handle_key(key) {
                PickerAction::None => {}
                PickerAction::Cancel => self.picker = None,
                PickerAction::Confirm(selection) => {
                    self.form.set_input(selection);
                    self.picker = None;
                }
            }
            return false;
        }

        self.error = None;
        match self.form.handle_key(key) {
            FormAction::None => false,
            FormAction::Quit => true,
            FormAction::PickInput => {
                match FilePicker::open(self.form.input()) {
                    Ok(picker) => self.picker = Some(picker),
                    Err(error) => self.error = Some(format!("Cannot open file picker: {error}")),
                }
                false
            }
        }
    }

    pub(crate) fn form(&self) -> &TranscodeForm {
        &self.form
    }

    pub(crate) fn picker(&self) -> Option<&FilePicker> {
        self.picker.as_ref()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
