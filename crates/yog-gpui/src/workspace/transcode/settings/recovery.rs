use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, StyledExt as _,
    button::Button,
    checkbox::Checkbox,
    form::{Field, Form},
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use yog_runtime::recovery::{RecoveryMode, RecoveryOptions};

use super::super::super::video::{Choice, selected};

/// Frontend controls only; runtime owns matching and persistence.
pub(super) struct RecoveryControls {
    mode: Choice,
    path: Entity<InputState>,
    batch: Choice,
    file: Choice,
    running: bool,
    keep_manual_skips: bool,
    error: Option<String>,
    _subscriptions: [Subscription; 3],
}

impl RecoveryControls {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut select = |items: Vec<&'static str>, index, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
            })
        };
        let mode = select(vec!["Automatic", "Restart", "Off"], 0, cx);
        let policies = vec!["Ask", "Abort", "Force", "Skip", "Restart"];
        let batch = select(policies.clone(), 0, cx);
        let file = select(policies, 0, cx);
        let subscriptions = [&mode, &batch, &file].map(|choice| {
            cx.subscribe(
                choice,
                |_, _, _: &SelectEvent<SearchableVec<&'static str>>, cx| cx.notify(),
            )
        });
        Self {
            mode,
            batch,
            file,
            path: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Automatic: output/.yog-state.jsonl")
            }),
            running: false,
            keep_manual_skips: true,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn options(&self, cx: &App) -> RecoveryOptions {
        let path = self.path.read(cx).value();
        RecoveryOptions {
            mode: match selected(&self.mode, cx) {
                "Automatic" => RecoveryMode::Automatic,
                "Restart" => RecoveryMode::Restart,
                "Off" => RecoveryMode::Off,
                _ => unreachable!(),
            },
            path: (!path.trim().is_empty()).then(|| PathBuf::from(path.trim())),
            batch_mismatch: selected(&self.batch, cx)
                .to_ascii_lowercase()
                .parse()
                .unwrap(),
            file_mismatch: selected(&self.file, cx)
                .to_ascii_lowercase()
                .parse()
                .unwrap(),
            keep_manual_skips: self.keep_manual_skips,
        }
    }

    pub(super) fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.running = running;
        cx.notify();
    }

    fn choose_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let response = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose state file".into()),
        });
        cx.spawn_in(window, async move |this, cx| match response.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    _ = this.update_in(cx, |this, window, cx| {
                        if this.running {
                            return;
                        }
                        this.path.update(cx, |input, cx| {
                            input.set_value(path.to_string_lossy().to_string(), window, cx)
                        });
                        this.error = None;
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            result => {
                _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not open state picker: {result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for RecoveryControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = selected(&self.mode, cx);
        let form = Form::new()
            .child(
                Field::new()
                    .label("Mode")
                    .child(Select::new(&self.mode).disabled(self.running).w_full()),
            )
            .when(mode != "Off", |form| {
                form.child(
                    Field::new().label("State file").child(
                        div()
                            .flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                Input::new(&self.path)
                                    .disabled(self.running)
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(
                                Button::new("browse-state")
                                    .label("Browse…")
                                    .disabled(self.running)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_path(window, cx)
                                    })),
                            ),
                    ),
                )
            })
            .when(mode == "Automatic", |form| {
                form.child(
                    Field::new()
                        .label("Batch mismatch")
                        .child(Select::new(&self.batch).disabled(self.running).w_full()),
                )
                .child(
                    Field::new()
                        .label("File mismatch")
                        .child(Select::new(&self.file).disabled(self.running).w_full()),
                )
                .child(
                    Field::new().child(
                        Checkbox::new("keep-manual-skips")
                            .label("Keep manually skipped tasks")
                            .checked(self.keep_manual_skips)
                            .disabled(self.running)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.keep_manual_skips = *checked;
                                cx.notify();
                            })),
                    ),
                )
            });
        div().flex().flex_col().gap_4()
            .child(div().font_medium().child("Recovery"))
            .child(form)
            .when(mode == "Automatic" && selected(&self.batch, cx) == "Force", |this| this.child(div().text_sm().text_color(cx.theme().warning)
                .child("Batch Force reuses recorded settings. Individual files are still checked; completed videos remain at their recorded paths.")))
            .when(mode == "Automatic" && selected(&self.file, cx) == "Force", |this| this.child(div().text_sm().text_color(cx.theme().warning)
                .child("File Force trusts changed or missing files. Missing outputs are not regenerated.")))
            .when(mode == "Restart", |this| this.child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Rebuild state. Existing outputs are not overwritten unless enabled separately.")))
            .when_some(self.error.clone(), |this, error| this.child(div().text_sm().text_color(cx.theme().danger).child(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt as _};
    use yog_runtime::recovery::MismatchPolicy;

    #[gpui_kit::test]
    fn manual_skip_recovery_choice_is_editable_before_a_run(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut controls = None;
        let handle = cx.open_window(size(px(520.), px(650.)), |window, cx| {
            let view = cx.new(|cx| RecoveryControls::new(window, cx));
            controls = Some(view.clone());
            Root::new(view, window, cx)
        });
        let controls = controls.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(controls.read(cx).options(cx).keep_manual_skips);
            window.click("keep-manual-skips", cx);
            assert!(!controls.read(cx).options(cx).keep_manual_skips);
            controls.update(cx, |controls, cx| controls.set_running(true, cx));
            window.render_frame(cx);
            window.click("keep-manual-skips", cx);
            assert!(!controls.read(cx).options(cx).keep_manual_skips);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn recovery_controls_map_choices_and_disable_while_running(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut controls = None;
        let handle = cx.open_window(size(px(500.), px(600.)), |window, cx| {
            let view = cx.new(|cx| RecoveryControls::new(window, cx));
            controls = Some(view.clone());
            Root::new(view, window, cx)
        });
        let controls = controls.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            controls.update(cx, |this, cx| {
                assert_eq!(this.options(cx).mode, RecoveryMode::Automatic);
                this.batch.update(cx, |choice, cx| {
                    choice.set_selected_value(&"Force", window, cx)
                });
                this.file.update(cx, |choice, cx| {
                    choice.set_selected_value(&"Skip", window, cx)
                });
                this.path.update(cx, |input, cx| {
                    input.set_value("old/state.jsonl", window, cx)
                });
                let options = this.options(cx);
                assert_eq!(options.batch_mismatch, MismatchPolicy::Force);
                assert_eq!(options.file_mismatch, MismatchPolicy::Skip);
                assert_eq!(
                    options.path.as_deref(),
                    Some(std::path::Path::new("old/state.jsonl"))
                );
                this.set_running(true, cx);
            });
            window.render_frame(cx);
            assert!(controls.read(cx).running);
            window.click("browse-state", cx);
            controls.update(cx, |this, cx| {
                this.set_running(false, cx);
                this.mode.update(cx, |choice, cx| {
                    choice.set_selected_value(&"Off", window, cx)
                });
                assert_eq!(this.options(cx).mode, RecoveryMode::Off);
            });
            window.render_frame(cx);
            assert!(window.try_find("browse-state").is_none());
        })
        .unwrap();
        assert!(!cx.did_prompt_for_paths());
    }
}
