use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_runtime::{RunOutcome, event::RunEvent};

use crate::{
    candidate::{CandidateAction, CandidateEditor},
    file_picker::{FilePicker, PickerAction},
    form::{CommandForm, FormAction, RunRequest},
    recovery::{RecoveryDialog, RecoveryDialogAction},
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
    recovery: Option<RecoveryDialog>,
    error: Option<String>,
}

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if let Some(dialog) = &mut self.recovery {
            if !dialog.visible && matches!(key.code, KeyCode::Char('r') | KeyCode::Enter) {
                dialog.visible = true;
                return AppAction::None;
            }
            if dialog.visible {
                match dialog.handle_key(key) {
                    RecoveryDialogAction::None => {}
                    RecoveryDialogAction::Close => dialog.visible = false,
                    RecoveryDialogAction::Cancel => {
                        self.run
                            .as_mut()
                            .expect("a recovery decision belongs to a run")
                            .request_cancel();
                        self.recovery = None;
                        return AppAction::Cancel;
                    }
                    RecoveryDialogAction::Respond => {
                        self.recovery.take().unwrap().respond();
                        self.run
                            .as_mut()
                            .expect("a recovery decision belongs to a run")
                            .stage = RunStage::Running;
                    }
                }
                return AppAction::None;
            }
        }
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
                RunStage::Running | RunStage::Waiting | RunStage::Cancelling
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if run.stage != RunStage::Cancelling {
                        run.request_cancel();
                        self.recovery = None;
                        AppAction::Cancel
                    } else {
                        AppAction::None
                    }
                }
                RunStage::Running | RunStage::Waiting | RunStage::Cancelling => AppAction::None,
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

    pub fn recovery(&self) -> Option<&RecoveryDialog> {
        self.recovery.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn handle_run_event(&mut self, event: RunEvent) {
        if let RunEvent::RecoveryDecisionRequired(request) = event {
            self.run
                .as_mut()
                .expect("runtime events require an active run")
                .stage = RunStage::Waiting;
            self.recovery = Some(RecoveryDialog::new(request));
            return;
        }
        self.run
            .as_mut()
            .expect("runtime events require an active run")
            .handle_event(event);
    }

    pub fn finish_run(&mut self, outcome: RunOutcome) {
        self.recovery = None;
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
            .is_some_and(|run| matches!(run.stage, RunStage::Running | RunStage::Cancelling))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn recovery_request() -> (
        yog_runtime::recovery::RecoveryRequest,
        tokio::sync::oneshot::Receiver<Result<yog_runtime::recovery::RecoveryDecision, String>>,
    ) {
        use yog_runtime::recovery::*;
        let (reply, response) = tokio::sync::oneshot::channel();
        (
            RecoveryRequest {
                state_path: "/out/.yog-state.jsonl".into(),
                input: "/in/movie.mkv".into(),
                output: Some("/out/movie.mkv".into()),
                scope: RecoveryScope::Batch,
                differences: vec![RecoveryDifference {
                    kind: RecoveryDifferenceKind::Container,
                    recorded: "mkv".into(),
                    current: "mp4".into(),
                }],
                overwrite_output: false,
                reply,
            },
            response,
        )
    }

    fn rendered(app: &App) -> String {
        use crate::ui::DrawApp;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
        terminal.draw(|frame| frame.draw(app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn recovery_request_is_shown_as_a_persistent_decision() {
        let mut app = App {
            run: Some(RunState::new(&yog_runtime::Operation::Transcode)),
            ..App::default()
        };
        let (request, mut response) = recovery_request();
        app.handle_run_event(RunEvent::RecoveryDecisionRequired(request));
        let screen = rendered(&app);
        assert!(screen.contains("Recovery settings differ"), "{screen}");
        assert!(screen.contains("Trust recorded results"), "{screen}");
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn dismissed_recovery_request_can_be_reopened_and_answered() {
        let mut app = App {
            run: Some(RunState::new(&yog_runtime::Operation::Transcode)),
            ..App::default()
        };
        let (request, mut response) = recovery_request();
        app.handle_run_event(RunEvent::RecoveryDecisionRequired(request));
        app.handle_key(key(KeyCode::Esc));
        assert!(rendered(&app).contains("Waiting for confirmation"));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Enter));
        let decision = response.try_recv().unwrap().unwrap();
        assert_eq!(
            decision.action,
            yog_runtime::recovery::RecoveryAction::Force
        );
        assert!(decision.apply_to_similar);
        assert_eq!(app.run().unwrap().stage, RunStage::Running);
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
