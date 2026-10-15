use gpui_kit::component::{
    ActiveTheme, Disableable, WindowExt as _,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState, NumberInput},
};
#[cfg(test)]
use gpui_kit::test::TestSupportExt as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use std::num::NonZeroUsize;

pub(super) struct SkipRequested {
    pub index: usize,
    pub count: NonZeroUsize,
}

struct SkipTarget {
    index: usize,
    name: String,
    vmaf: bool,
    finished: bool,
}

pub(super) struct SkipDialog {
    target: Option<SkipTarget>,
    input: Entity<InputState>,
    open: bool,
    _input_subscription: Subscription,
}

impl EventEmitter<SkipRequested> for SkipDialog {}

impl SkipDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("1")
                .min(1.)
                .step(1.)
        });
        let input_subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            target: None,
            input,
            open: false,
            _input_subscription: input_subscription,
        }
    }

    pub(super) fn open(
        &mut self,
        index: usize,
        name: String,
        vmaf: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open {
            return;
        }
        self.target = Some(SkipTarget {
            index,
            name,
            vmaf,
            finished: false,
        });
        self.input
            .update(cx, |input, cx| input.set_value("1", window, cx));
        self.open = true;
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            let on_close = view.downgrade();
            let confirm = view.clone();
            let width = (cx.theme().font_size * 30.)
                .min(window.viewport_size().width - cx.theme().font_size * 4.);
            dialog
                .title("Skip tasks")
                .width(width)
                .child(view.clone())
                .on_ok(move |_, window, cx| {
                    confirm.update(cx, |this, cx| this.submit(window, cx));
                    false // submit owns validation and closes only after accepting a count.
                })
                .on_close(move |_, _, cx| {
                    let _ = on_close.update(cx, |this, cx| {
                        this.open = false;
                        this.target = None;
                        cx.notify();
                    });
                })
        });
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(super) fn invalidate(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(target) = &mut self.target
            && target.index == index
        {
            target.finished = true;
            cx.notify();
        }
    }

    pub(super) fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.target = None;
        if self.open {
            self.open = false;
            window.close_dialog(cx);
        }
        cx.notify();
    }

    pub(super) fn count(&self, cx: &App) -> Option<NonZeroUsize> {
        self.input.read(cx).value().parse().ok()
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = &self.target
            && !target.finished
            && let Some(count) = self.count(cx)
        {
            let index = target.index;
            self.clear(window, cx);
            cx.emit(SkipRequested { index, count });
        }
    }
}

impl Render for SkipDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(target) = &self.target else {
            return div();
        };
        let valid = self.count(cx).is_some();
        let input = div().id("skip-count").child(NumberInput::new(&self.input));
        #[cfg(test)]
        let input = input.test_support();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .min_w_0()
            .child(div().text_sm().child(target.name.clone()))
            .child(div().flex().flex_col().gap_2().child("Count").child(input))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Includes the current task. Already completed tasks are not counted."),
            )
            .when(target.vmaf, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Current video is complete; only its VMAF calculation will be skipped.",
                        ),
                )
            })
            .when(target.finished, |this| {
                this.child(div().text_sm().text_color(cx.theme().warning).child(
                    "This task has finished. Close this dialog and select the current task.",
                ))
            })
            .when(!valid, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child("Enter a positive whole number."),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("close-skip-tasks")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                    )
                    .child(
                        Button::new("confirm-skip-tasks")
                            .primary()
                            .label("Skip")
                            .disabled(target.finished || !valid)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
