use gpui_kit::component::{
    ActiveTheme, IndexPath, Sizable as _,
    button::{Button, ButtonVariants},
    searchable_list::SearchableListItem,
    select::{Select, SelectEvent, SelectState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ListState, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use yog_runtime::event::{RunPhase, SkipReason, TaskResult, TaskStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterItem {
    All,
    Status(TaskStatus),
    Skip(SkipReason),
    Failure { phase: RunPhase, timed_out: bool },
}

impl SearchableListItem for FilterItem {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::All => "All".into(),
            Self::Status(status) => status.label().into(),
            Self::Skip(reason) => reason.label().into(),
            Self::Failure { phase, timed_out } => {
                if *timed_out {
                    format!("{} · Timed out", phase.label()).into()
                } else {
                    phase.label().into()
                }
            }
        }
    }

    fn value(&self) -> &Self {
        self
    }
}

#[derive(Clone, Copy)]
struct Classification {
    status: TaskStatus,
    reason: FilterItem,
}

impl Classification {
    fn from_result(result: &TaskResult) -> Self {
        Self {
            status: result.status(),
            reason: match result {
                TaskResult::Skipped { reason, .. } => FilterItem::Skip(*reason),
                TaskResult::Failed {
                    phase, timed_out, ..
                } => FilterItem::Failure {
                    phase: *phase,
                    timed_out: *timed_out,
                },
                TaskResult::Success | TaskResult::Cancelled => FilterItem::All,
            },
        }
    }
}

/// Owns only classifications and visible source indices, never row payloads.
pub(super) struct ResultFilter {
    status: Entity<SelectState<Vec<FilterItem>>>,
    reason: Entity<SelectState<Vec<FilterItem>>>,
    classifications: Vec<Classification>,
    observed_reasons: Vec<Classification>,
    visible: Vec<usize>,
    newest_first: bool,
    list: ListState,
    _subscriptions: Vec<Subscription>,
}

impl ResultFilter {
    pub(super) fn new(
        include_skipped: bool,
        newest_first: bool,
        list: ListState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut statuses = vec![
            FilterItem::All,
            FilterItem::Status(TaskStatus::Success),
            FilterItem::Status(TaskStatus::Failure),
            FilterItem::Status(TaskStatus::Cancelled),
        ];
        if include_skipped {
            statuses.push(FilterItem::Status(TaskStatus::Skipped));
        }
        let status =
            cx.new(|cx| SelectState::new(statuses, Some(IndexPath::default().row(0)), window, cx));
        let reason = cx.new(|cx| {
            SelectState::new(
                vec![FilterItem::All],
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let status_subscription = cx.subscribe_in(
            &status,
            window,
            |this, _, _: &SelectEvent<Vec<FilterItem>>, window, cx| {
                this.reason.update(cx, |reason, cx| {
                    reason.set_selected_value(&FilterItem::All, window, cx)
                });
                this.refresh_reasons(window, cx);
                this.rebuild(cx);
            },
        );
        let reason_subscription = cx
            .subscribe(&reason, |this, _, _: &SelectEvent<Vec<FilterItem>>, cx| {
                this.rebuild(cx)
            });
        Self {
            status,
            reason,
            classifications: Vec::new(),
            observed_reasons: Vec::new(),
            visible: Vec::new(),
            newest_first,
            list,
            _subscriptions: vec![status_subscription, reason_subscription],
        }
    }

    fn matches(&self, classification: Classification, cx: &Context<Self>) -> bool {
        let status = *self
            .status
            .read(cx)
            .selected_value()
            .expect("status filter has a selection");
        let reason = *self
            .reason
            .read(cx)
            .selected_value()
            .expect("reason filter has a selection");
        (status == FilterItem::All || status == FilterItem::Status(classification.status))
            && (reason == FilterItem::All || reason == classification.reason)
    }

    fn refresh_reasons(&self, window: &mut Window, cx: &mut Context<Self>) {
        let status = *self
            .status
            .read(cx)
            .selected_value()
            .expect("status filter has a selection");
        let mut reasons = vec![FilterItem::All];
        reasons.extend(
            self.observed_reasons
                .iter()
                .filter(|item| {
                    status == FilterItem::All || status == FilterItem::Status(item.status)
                })
                .map(|item| item.reason),
        );
        self.reason.update(cx, |reason, cx| {
            let selected = *reason
                .selected_value()
                .expect("reason filter has a selection");
            reason.set_items(reasons, window, cx);
            reason.set_selected_value(&selected, window, cx);
        });
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.visible = self
            .classifications
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, item)| self.matches(item, cx).then_some(index))
            .collect();
        self.list.reset(self.visible.len() + 1);
        cx.notify();
    }

    pub(super) fn append(
        &mut self,
        result: &TaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let classification = Classification::from_result(result);
        let index = self.classifications.len();
        self.classifications.push(classification);
        if classification.reason != FilterItem::All
            && !self
                .observed_reasons
                .iter()
                .any(|item| item.reason == classification.reason)
        {
            self.observed_reasons.push(classification);
            self.refresh_reasons(window, cx);
        }
        if self.matches(classification, cx) {
            self.visible.push(index);
            let position = if self.newest_first {
                1
            } else {
                self.list.item_count()
            };
            self.list.splice(position..position, 1);
        }
        self.list.remeasure_items(0..1);
        cx.notify();
    }

    pub(super) fn source_index(&self, row: usize) -> usize {
        self.visible[if self.newest_first {
            self.visible.len() - row - 1
        } else {
            row
        }]
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            status.set_selected_value(&FilterItem::All, window, cx)
        });
        self.reason.update(cx, |reason, cx| {
            reason.set_selected_value(&FilterItem::All, window, cx)
        });
        self.refresh_reasons(window, cx);
        self.rebuild(cx);
    }

    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.classifications.clear();
        self.observed_reasons.clear();
        self.clear_filters(window, cx);
    }
}

impl Render for ResultFilter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        Select::new(&self.status)
                            .id("result-status-filter")
                            .accessibility_label("Result status")
                            .title_prefix("Status: ")
                            .small(),
                    )
                    .child(
                        Select::new(&self.reason)
                            .id("result-reason-filter")
                            .accessibility_label("Result reason")
                            .title_prefix("Reason: ")
                            .small(),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} of {} results",
                                self.visible.len(),
                                self.classifications.len()
                            )),
                    ),
            )
            .when(
                !self.classifications.is_empty() && self.visible.is_empty(),
                |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No matching results"),
                            )
                            .child(
                                Button::new("clear-result-filters")
                                    .ghost()
                                    .small()
                                    .label("Clear filters")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_filters(window, cx)
                                    })),
                            ),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        ListAlignment, TestAppContext, component::Root, px, size, test::TestWindowExt as _,
    };

    fn failure(phase: RunPhase, timed_out: bool) -> TaskResult {
        TaskResult::Failed {
            phase,
            timed_out,
            message: "tool failed".into(),
        }
    }

    #[gpui_kit::test]
    fn status_menu_filters_existing_and_incoming_results(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut filter = None;
        let handle = cx.open_window(size(px(800.), px(500.)), |window, cx| {
            let view = cx.new(|cx| {
                ResultFilter::new(
                    true,
                    false,
                    ListState::new(1, ListAlignment::Top, px(300.)),
                    window,
                    cx,
                )
            });
            filter = Some(view.clone());
            Root::new(view, window, cx)
        });
        let filter = filter.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            filter.update(cx, |filter, cx| {
                filter.append(&TaskResult::Success, window, cx);
                filter.append(&failure(RunPhase::Transcoding, false), window, cx);
                filter.append(&TaskResult::Cancelled, window, cx);
            });
            window.render_frame(cx);
            window.click("result-status-filter", cx);
            window.render_frame(cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(filter.read(cx).visible, [1]);
            filter.update(cx, |filter, cx| {
                filter.append(&TaskResult::Success, window, cx);
                filter.append(&failure(RunPhase::Publishing, false), window, cx);
            });
            assert_eq!(filter.read(cx).visible, [1, 4]);
            assert_eq!(filter.read(cx).list.item_count(), 3);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn reason_menu_distinguishes_timeout_and_resets_when_status_changes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut filter = None;
        let handle = cx.open_window(size(px(800.), px(500.)), |window, cx| {
            let view = cx.new(|cx| {
                ResultFilter::new(
                    true,
                    false,
                    ListState::new(1, ListAlignment::Top, px(300.)),
                    window,
                    cx,
                )
            });
            filter = Some(view.clone());
            Root::new(view, window, cx)
        });
        let filter = filter.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            filter.update(cx, |filter, cx| {
                filter.append(&failure(RunPhase::Probing, true), window, cx);
                filter.append(&failure(RunPhase::Probing, false), window, cx);
                filter.append(
                    &TaskResult::Skipped {
                        reason: SkipReason::NoVideo,
                        message: "audio only".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.click("result-reason-filter", cx);
            window.render_frame(cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(filter.read(cx).visible, [0]);
            filter.update(cx, |filter, cx| {
                filter.append(&failure(RunPhase::Probing, true), window, cx)
            });
            assert_eq!(filter.read(cx).visible, [0, 3]);
            window.render_frame(cx);
            window.click("result-status-filter", cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(filter.read(cx).visible.is_empty());
            assert_eq!(
                filter.read(cx).reason.read(cx).selected_value(),
                Some(&FilterItem::All)
            );
            window.render_frame(cx);
            assert!(window.find("clear-result-filters").visible());
            window.click("clear-result-filters", cx);
            assert_eq!(filter.read(cx).visible, [0, 1, 2, 3]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn resetting_for_a_new_run_clears_results_and_both_filters(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.open_window(size(px(800.), px(500.)), |window, cx| {
            let filter = cx.new(|cx| {
                ResultFilter::new(
                    false,
                    true,
                    ListState::new(1, ListAlignment::Top, px(300.)),
                    window,
                    cx,
                )
            });
            filter.update(cx, |filter, cx| {
                filter.append(&failure(RunPhase::Predicting, false), window, cx);
                filter.append(&TaskResult::Success, window, cx);
                assert_eq!(filter.source_index(0), 1);
                filter.status.update(cx, |status, cx| {
                    status.set_selected_value(&FilterItem::Status(TaskStatus::Failure), window, cx)
                });
                filter.reason.update(cx, |reason, cx| {
                    reason.set_selected_value(
                        &FilterItem::Failure {
                            phase: RunPhase::Predicting,
                            timed_out: false,
                        },
                        window,
                        cx,
                    )
                });
                filter.reset(window, cx);
                assert!(filter.visible.is_empty());
                assert!(filter.classifications.is_empty());
                assert_eq!(
                    filter.status.read(cx).selected_value(),
                    Some(&FilterItem::All)
                );
                assert_eq!(
                    filter.reason.read(cx).selected_value(),
                    Some(&FilterItem::All)
                );
                filter.append(&TaskResult::Success, window, cx);
                assert_eq!(filter.visible, [0]);
                assert_eq!(filter.list.item_count(), 2);
            });
            Root::new(filter, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
}
