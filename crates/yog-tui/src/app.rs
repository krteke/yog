use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_runtime::{RunOutcome, event::RunEvent};

use crate::{
    candidate::{CandidateAction, CandidateEditor},
    file_picker::{FilePicker, PickerAction},
    form::{CommandForm, FormAction, RunRequest},
    run::{RunStage, RunState},
};

pub enum AppAction {
    None,
    Quit,
    Start(Box<RunRequest>),
    Cancel,
}

#[derive(Default)]
pub struct App {
    form: CommandForm,
    candidate_editor: Option<CandidateEditor>,
    picker: Option<FilePicker>,
    run: Option<RunState>,
    error: Option<String>,
}

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if let Some(editor) = &mut self.candidate_editor {
            match editor.handle_key(key) {
                CandidateAction::None => {}
                CandidateAction::Cancel => self.candidate_editor = None,
                CandidateAction::Confirm(candidates) => {
                    self.form.set_candidates(candidates);
                    self.candidate_editor = None;
                }
            }
            return AppAction::None;
        }

        if let Some(picker) = &mut self.picker {
            match picker.handle_key(key) {
                PickerAction::None => {}
                PickerAction::Cancel => self.picker = None,
                PickerAction::Confirm(selection) => {
                    self.form.set_input(selection);
                    self.picker = None;
                }
            }
            return AppAction::None;
        }

        if let Some(run) = &mut self.run {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    run.toggle_panel();
                    return AppAction::None;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    run.move_panel_cursor(1);
                    return AppAction::None;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    run.move_panel_cursor(-1);
                    return AppAction::None;
                }
                KeyCode::Char('g') | KeyCode::Home => {
                    run.move_panel_cursor_to(false);
                    return AppAction::None;
                }
                KeyCode::Char('G') | KeyCode::End => {
                    run.move_panel_cursor_to(true);
                    return AppAction::None;
                }
                _ => {}
            }
            return match run.stage {
                RunStage::Finished(_) => match key.code {
                    KeyCode::Char('q') => AppAction::Quit,
                    KeyCode::Enter | KeyCode::Esc => {
                        self.run = None;
                        AppAction::None
                    }
                    _ => AppAction::None,
                },
                RunStage::Running | RunStage::Cancelling
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if run.stage == RunStage::Running {
                        run.request_cancel();
                        AppAction::Cancel
                    } else {
                        AppAction::None
                    }
                }
                RunStage::Running | RunStage::Cancelling => AppAction::None,
            };
        }

        self.error = None;
        match self.form.handle_key(key) {
            FormAction::None => AppAction::None,
            FormAction::Quit => AppAction::Quit,
            FormAction::PickInput => {
                match FilePicker::open(self.form.input()) {
                    Ok(picker) => self.picker = Some(picker),
                    Err(error) => self.error = Some(format!("Cannot open file picker: {error}")),
                }
                AppAction::None
            }
            FormAction::EditCandidates => {
                self.candidate_editor = Some(self.form.candidate_editor());
                AppAction::None
            }
            FormAction::Submit => match self.form.build_request() {
                Ok(request) => {
                    self.run = Some(RunState::new(&request.command.operation));
                    AppAction::Start(Box::new(request))
                }
                Err(error) => {
                    self.error = Some(error);
                    AppAction::None
                }
            },
        }
    }

    pub fn form(&self) -> &CommandForm {
        &self.form
    }

    pub fn picker(&self) -> Option<&FilePicker> {
        self.picker.as_ref()
    }

    pub fn candidate_editor(&self) -> Option<&CandidateEditor> {
        self.candidate_editor.as_ref()
    }

    pub fn run(&self) -> Option<&RunState> {
        self.run.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn handle_run_event(&mut self, event: RunEvent) {
        self.run
            .as_mut()
            .expect("runtime events require an active run")
            .handle_event(event);
    }

    pub fn finish_run(&mut self, outcome: RunOutcome) {
        self.run
            .as_mut()
            .expect("runtime completion requires an active run")
            .finish(outcome);
    }

    pub fn tick(&mut self) {
        self.run
            .as_mut()
            .expect("animation ticks require an active run")
            .tick();
    }

    pub fn is_animating(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| !matches!(run.stage, RunStage::Finished(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn running_screen_cancels_once_then_returns_after_completion() {
        let mut app = App {
            run: Some(RunState::new(&yog_runtime::Operation::Transcode)),
            ..App::default()
        };

        assert!(matches!(
            app.handle_key(key(KeyCode::Char('q'))),
            AppAction::Cancel
        ));
        assert_eq!(app.run().unwrap().stage, RunStage::Cancelling);
        assert!(matches!(
            app.handle_key(key(KeyCode::Char('q'))),
            AppAction::None
        ));

        app.finish_run(RunOutcome::Cancelled);
        assert_eq!(
            app.run().unwrap().stage,
            RunStage::Finished(yog_runtime::RunStatus::Cancelled)
        );
        assert!(matches!(
            app.handle_key(key(KeyCode::Enter)),
            AppAction::None
        ));
        assert!(app.run().is_none());
    }

    #[test]
    fn candidate_edits_are_applied_only_after_confirmation() {
        let mut app = App::default();
        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.candidate_editor().is_some());

        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Esc));
        assert!(app.candidate_editor().is_some());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.candidate_editor().is_none());
        assert_eq!(
            app.form().value(crate::form::Field::Candidates),
            "Single encoder"
        );

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('s')));
        assert!(app.candidate_editor().is_none());
        assert_eq!(
            app.form().value(crate::form::Field::Candidates),
            "2 configured"
        );

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('l')));
        app.handle_key(key(KeyCode::Enter));
        assert!(
            app.candidate_editor().unwrap().candidates()[0]
                .summary()
                .contains("VAAPI")
        );
        app.handle_key(key(KeyCode::Char('s')));

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('s')));
        assert_eq!(
            app.form().value(crate::form::Field::Candidates),
            "Single encoder"
        );
        assert!(
            app.form()
                .source_fields()
                .contains(&crate::form::Field::Decoder)
        );
    }
}
