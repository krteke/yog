use gpui_kit::component::{
    ActiveTheme, StyledExt as _, WindowExt as _, button::Button, checkbox::Checkbox,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Window, div,
};
use yog_runtime::recovery::{RecoveryAction, RecoveryRequest};

pub struct RecoveryAnswered;

pub struct RecoveryDialog {
    request: Option<RecoveryRequest>,
    apply_to_similar: bool,
    open: bool,
}

impl EventEmitter<RecoveryAnswered> for RecoveryDialog {}

impl RecoveryDialog {
    pub fn new() -> Self {
        Self {
            request: None,
            apply_to_similar: false,
            open: false,
        }
    }

    pub fn is_waiting(&self) -> bool {
        self.request.is_some()
    }

    pub fn show(&mut self, request: RecoveryRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.request = Some(request);
        self.apply_to_similar = false;
        self.open(window, cx);
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = &self.request else {
            return;
        };
        if self.open {
            return;
        }
        let title = request.title();
        let view = cx.entity();
        self.open = true;
        window.open_dialog(cx, move |dialog, window, cx| {
            let on_close = view.downgrade();
            let width = (cx.theme().font_size * 44.0)
                .min(window.viewport_size().width - cx.theme().font_size * 4.0);
            dialog
                .title(title)
                .width(width)
                .overlay_closable(false)
                .on_close(move |_, _, cx| {
                    let _ = on_close.update(cx, |view, cx| {
                        view.open = false;
                        cx.notify();
                    });
                })
                .child(view.clone())
        });
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request = None;
        if self.open {
            self.open = false;
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn answer(&mut self, action: RecoveryAction, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(request) = self.request.take() {
            request.respond(action, self.apply_to_similar);
            self.clear(window, cx);
            cx.emit(RecoveryAnswered);
        }
    }
}

impl Render for RecoveryDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(request) = &self.request else {
            return div();
        };
        let differences = request
            .differences
            .iter()
            .map(|difference| {
                let column = |label, value: &str| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(label),
                        )
                        .child(div().text_sm().child(value.to_owned()))
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pb_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_medium().child(difference.kind.to_string()))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_4()
                            .child(column("Recorded", &difference.recorded))
                            .child(column("Current", &difference.current)),
                    )
            })
            .collect::<Vec<_>>();
        let choices = RecoveryAction::ALL
            .into_iter()
            .map(|action| {
                let id = match action {
                    RecoveryAction::Abort => "recovery-abort",
                    RecoveryAction::Force => "recovery-force",
                    RecoveryAction::Skip => "recovery-skip",
                    RecoveryAction::Restart => "recovery-restart",
                };
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        Button::new(id)
                            .label(request.action_label(action))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.answer(action, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(request.action_detail(action)),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .min_w_0()
            .child(div().text_sm().child(request.input.display().to_string()))
            .when_some(request.output.as_ref(), |this, output| {
                this.child(
                    div()
                        .text_sm()
                        .child(format!("Recorded output: {}", output.display())),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("State: {}", request.state_path.display())),
            )
            .children(differences)
            .child(
                Checkbox::new("recovery-apply-similar")
                    .label("Apply to later differences of the same kind in this run")
                    .checked(self.apply_to_similar)
                    .on_change(cx.listener(|view, checked, _, cx| {
                        view.apply_to_similar = *checked;
                        cx.notify();
                    })),
            )
            .children(choices)
    }
}
