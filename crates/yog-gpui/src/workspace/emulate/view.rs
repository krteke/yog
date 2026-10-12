use super::*;
use gpui_kit::InteractiveElement as _;

impl EmulatePage {
    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (title, tag, state) = match self.stage {
            Stage::Running => (execution::phase_label(self.phase), Tag::info(), "Running"),
            Stage::Cancelling => ("Cancelling emulation", Tag::warning(), "Cancelling"),
            Stage::Finished(RunStatus::Success) => ("Chart generated", Tag::success(), "Completed"),
            Stage::Finished(RunStatus::Failure) => ("Emulation failed", Tag::danger(), "Failed"),
            Stage::Finished(RunStatus::Cancelled) => {
                ("Emulation cancelled", Tag::secondary(), "Cancelled")
            }
            Stage::Idle => unreachable!(),
        };
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        let percent =
            (self.total > 0).then(|| self.points.len() as f32 / self.total as f32 * 100.0);
        div()
            .min_w_0()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .text_lg()
                            .font_medium()
                            .truncate()
                            .child(title),
                    )
                    .child(tag.small().outline().child(state)),
            )
            .when(active, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            ProgressBar::new("emulate-progress")
                                .loading(percent.is_none())
                                .value(percent.unwrap_or_default())
                                .flex_1(),
                        )
                        .when_some(percent, |this, value| {
                            this.child(div().text_sm().child(format!("{value:.0}%")))
                        }),
                )
            })
            .when_some(self.current.as_ref(), |this, point| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!(
                            "{} {} · {}",
                            point.parameter, point.quality, point.label
                        )),
                )
            })
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} succeeded · {} failed · {} total{}",
                        self.succeeded,
                        self.failed,
                        self.total,
                        self.elapsed
                            .or_else(|| self.started.map(|start| start.elapsed()))
                            .map_or(String::new(), |elapsed| format!(
                                " · {} elapsed",
                                components::format_duration(elapsed)
                            ))
                    )),
            )
            .when(!self.chart_paths.is_empty(), |this| {
                this.child(div().flex().flex_wrap().gap_2().children(
                    self.chart_paths.iter().cloned().map(|path| {
                        let name = path
                            .file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                            .into_owned();
                        Button::new(format!("reveal-chart-{}", name))
                            .outline()
                            .label(format!("Reveal {name}"))
                            .on_click(move |_, _, cx| cx.reveal_path(&path))
                    }),
                ))
            })
    }

    fn render_point(&self, point: &Point, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let (state, tag) = match point.result.status() {
            TaskStatus::Success => ("Predicted", Tag::success()),
            TaskStatus::Failure => ("Failed", Tag::danger()),
            TaskStatus::Cancelled => ("Cancelled", Tag::secondary()),
            TaskStatus::Skipped => unreachable!("emulation points do not skip"),
        };
        let row = div()
            .min_w_0()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().min_w_0().font_medium().child(match point.candidate {
                        Some(candidate) => format!(
                            "Candidate {candidate} · {} {}",
                            point.parameter, point.quality
                        ),
                        None => format!("{} {}", point.parameter, point.quality),
                    }))
                    .child(tag.small().outline().child(state)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(point.label.clone()),
            )
            .when_some(point.vmaf, |this, vmaf| {
                this.child(div().text_sm().child(format!(
                        "VMAF {vmaf:.2} · {}",
                        point
                            .bytes
                            .map_or("Unknown size".into(), components::format_size)
                    )))
            })
            .when_some(point.result.message().map(str::to_owned), |this, error| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(format!("{}: {error}", point.result)),
                )
            });
        let row = row.id(format!(
            "emulate-point-{}-{}",
            point.candidate.unwrap_or(0),
            point.quality
        ));
        #[cfg(test)]
        let row = row.test_support();
        row.into_any_element()
    }

    pub fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let results = components::virtual_list(
            "emulate-results",
            self.work_list.clone(),
            cx.theme().font_size * 10.,
            move |index, _, cx| {
                view.update(cx, |page, cx| {
                    if index == 0 {
                        div()
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(page.source.clone())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(div().text_lg().font_medium().child("Results"))
                                    .when(matches!(page.stage, Stage::Idle), |this| {
                                        this.child(components::empty_results(cx))
                                    })
                                    .when(!matches!(page.stage, Stage::Idle), |this| {
                                        this.child(page.render_summary(cx))
                                            .child(page.result_filter.clone())
                                    })
                                    .when_some(page.chart_paths.first().cloned(), |this, path| {
                                        let image =
                                            img(path).object_fit(ObjectFit::Contain).size_full();
                                        #[cfg(test)]
                                        let image = image.id("emulate-chart-image").test_support();
                                        let preview = div()
                                            .min_w_0()
                                            .rounded(cx.theme().radius)
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .bg(cx.theme().group_box)
                                            .p_3()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .aspect_ratio(page.chart_aspect_ratio)
                                                    .child(image),
                                            );
                                        #[cfg(test)]
                                        let preview =
                                            preview.id("emulate-chart-preview").test_support();
                                        this.child(preview)
                                    }),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .px_6()
                            .pb_4()
                            .child(page.render_point(
                                &page.points[page.result_filter.read(cx).source_index(index - 1)],
                                cx,
                            ))
                            .into_any_element()
                    }
                })
            },
        );
        components::work_and_messages(
            "emulate-work-and-messages",
            results.into_any_element(),
            self.messages.clone(),
            cx,
        )
    }

    fn render_candidate(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        div()
            .min_w_0()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .font_medium()
                            .child(format!("Candidate {}", index + 2)),
                    )
                    .child(
                        Button::new(format!("remove-candidate-{index}"))
                            .ghost()
                            .small()
                            .label("Remove")
                            .disabled(active)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_candidate(index, cx)),
                            ),
                    ),
            )
            .child(self.candidates[index].clone())
    }

    fn render_inspector_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let candidate_mode = !self.candidates.is_empty();
        let output = Form::new()
            .child(components::path_field(
                "PNG chart",
                "browse-png",
                &self.png,
                cx.listener(|this, _, window, cx| {
                    this.choose_destination(Destination::Png, window, cx)
                }),
            ))
            .child(components::path_field(
                "SVG chart",
                "browse-svg",
                &self.svg,
                cx.listener(|this, _, window, cx| {
                    this.choose_destination(Destination::Svg, window, cx)
                }),
            ))
            .child(components::path_field(
                "Report (optional)",
                "browse-emulate-report",
                &self.report,
                cx.listener(|this, _, window, cx| {
                    this.choose_destination(Destination::Report, window, cx)
                }),
            ))
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("emulate-overwrite")
                        .label("Overwrite charts")
                        .checked(self.overwrite_chart)
                        .on_change(cx.listener(|this, next, _, cx| {
                            this.overwrite_chart = *next;
                            cx.notify();
                        })),
                ),
            )
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("emulate-overwrite-report")
                        .label("Overwrite report")
                        .checked(self.overwrite_report)
                        .on_change(cx.listener(|this, next, _, cx| {
                            this.overwrite_report = *next;
                            cx.notify();
                        })),
                ),
            );
        div()
            .p_5()
            .track_focus(&self.inspector_focus)
            .flex()
            .flex_col()
            .gap_5()
            .child(div().font_medium().child("Chart output"))
            .child(output)
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .pt_4()
                    .font_medium()
                    .child("Encoding"),
            )
            .child(
                Form::new().child(
                    Field::new()
                        .label("Container")
                        .child(Select::new(&self.container).w_full()),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when(candidate_mode, |this| {
                        this.rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .p_3()
                            .child(div().font_medium().child("Candidate 1"))
                    })
                    .child(self.video.clone())
                    .child(
                        Form::new().child(
                            Field::new()
                                .label(format!(
                                    "Quality points ({})",
                                    self.video.read(cx).quality_parameter(cx)
                                ))
                                .child(Input::new(&self.quality_points)),
                        ),
                    ),
            )
    }

    pub fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        self.candidate_list.remeasure_items(0..1);
        let view = cx.entity();
        let body = components::virtual_list(
            "emulate-candidates",
            self.candidate_list.clone(),
            cx.theme().font_size * 40.,
            move |index, _, cx| {
                view.update(cx, |page, cx| {
                    if index == 0 {
                        page.render_inspector_header(cx).into_any_element()
                    } else if index <= page.candidates.len() {
                        div()
                            .px_5()
                            .pb_5()
                            .child(page.render_candidate(index - 1, cx))
                            .into_any_element()
                    } else {
                        div()
                            .px_5()
                            .pb_5()
                            .child(
                                Button::new("add-candidate")
                                    .outline()
                                    .label(if !page.candidates.is_empty() {
                                        "Add candidate"
                                    } else {
                                        "Compare candidates"
                                    })
                                    .disabled(matches!(
                                        page.stage,
                                        Stage::Running | Stage::Cancelling
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_candidate(window, cx)
                                    })),
                            )
                            .into_any_element()
                    }
                })
            },
        );
        let ready = self.source.read(cx).valid_path().is_some()
            && !self.source.read(cx).is_directory()
            && (!self.png.read(cx).value().trim().is_empty()
                || !self.svg.read(cx).value().trim().is_empty());
        let footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.destination_error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(div().flex().justify_end().child(if active {
                Button::new("cancel-emulate")
                    .danger()
                    .label(if matches!(self.stage, Stage::Cancelling) {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    })
                    .disabled(matches!(self.stage, Stage::Cancelling))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
            } else {
                Button::new("start-emulate")
                    .primary()
                    .label("Generate chart")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
            }));
        components::inspector(body.into_any_element(), footer.into_any_element(), cx)
    }
}
