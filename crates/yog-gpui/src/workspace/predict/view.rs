use super::*;

impl PredictPage {
    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (title, tag, state) = match self.stage {
            Stage::Running => (execution::phase_label(self.phase), Tag::info(), "Running"),
            Stage::Cancelling => ("Cancelling prediction", Tag::warning(), "Cancelling"),
            Stage::Finished(RunStatus::Success) => {
                ("Prediction complete", Tag::success(), "Completed")
            }
            Stage::Finished(RunStatus::Failure) => ("Prediction failed", Tag::danger(), "Failed"),
            Stage::Finished(RunStatus::Cancelled) => {
                ("Prediction cancelled", Tag::secondary(), "Cancelled")
            }
            Stage::Idle => unreachable!(),
        };
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        div()
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
                    ProgressBar::new("predict-progress")
                        .loading(true)
                        .accessibility_label("Prediction progress")
                        .w_full(),
                )
            })
            .when(active, |this| {
                this.child(
                    div()
                        .min_w_0()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "Task {}/{} · {}",
                            self.task_index,
                            self.task_total,
                            self.current_name
                                .as_ref()
                                .and_then(|path| path.file_name())
                                .map_or("Preparing", |name| name.to_str().unwrap_or("Video"))
                        )),
                )
            })
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} completed · {} failed · {} skipped · {} total{}",
                        self.completed,
                        self.failed,
                        self.skipped,
                        self.task_total,
                        self.elapsed
                            .or_else(|| self.started.map(|start| start.elapsed()))
                            .map_or(String::new(), |elapsed| format!(
                                " · {} elapsed",
                                components::format_duration(elapsed)
                            )),
                    )),
            )
    }

    fn render_result(&self, row: &ResultRow, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let name = row
            .path
            .file_name()
            .unwrap_or(row.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let prediction = row.prediction.as_ref();
        div()
            .min_w_0()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .font_medium()
                            .truncate()
                            .child(name),
                    )
                    .child(if prediction.is_some() {
                        Tag::success().small().outline().child("Predicted")
                    } else {
                        Tag::danger().small().outline().child("Failed")
                    }),
            )
            .when_some(prediction, |this, prediction| {
                this.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_5()
                        .child(metric(
                            "VMAF",
                            format!("{:.2}", prediction.quality.vmaf.value),
                            theme.muted_foreground,
                        ))
                        .child(metric(
                            "Estimated size",
                            components::format_size(prediction.output_bytes.value),
                            theme.muted_foreground,
                        ))
                        .child(metric(
                            "Encode time",
                            components::format_duration(Duration::from_secs_f64(
                                prediction.transcode_seconds.value.max(0.0),
                            )),
                            theme.muted_foreground,
                        ))
                        .child(metric(
                            "Speed",
                            format!("{:.2}×", prediction.speed.value),
                            theme.muted_foreground,
                        )),
                )
                .when_some(prediction.quality.ssim, |this, value| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("SSIM {:.4}", value.value)),
                    )
                })
                .when_some(prediction.quality.psnr_y_db, |this, value| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("PSNR-Y {:.2} dB", value.value)),
                    )
                })
            })
            .when_some(row.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(row.path.display().to_string()),
            )
            .into_any_element()
    }

    pub fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let results = div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(self.source.clone())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_lg().font_medium().child("Results"))
                            .when(matches!(self.stage, Stage::Idle), |this| {
                                this.child(components::empty_results(cx))
                            })
                            .when(!matches!(self.stage, Stage::Idle), |this| {
                                this.child(self.render_summary(cx))
                            })
                            .children(self.results.iter().map(|row| self.render_result(row, cx))),
                    ),
            );
        components::work_and_messages(
            "predict-work-and-messages",
            results.into_any_element(),
            self.messages.clone(),
            cx,
        )
    }

    pub fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rate = selected(&self.rate, cx);
        let mut form = Form::new()
            .child(
                Field::new()
                    .label("Container")
                    .child(Select::new(&self.container).w_full()),
            )
            .child(
                Field::new()
                    .label("Rate control")
                    .child(Select::new(&self.rate).w_full()),
            );
        match rate {
            "Quality" => {
                form = form.child(
                    Field::new()
                        .label(format!(
                            "Quality ({})",
                            self.video.read(cx).quality_parameter(cx)
                        ))
                        .child(Input::new(&self.quality)),
                )
            }
            "Bitrate" => {
                form = form.child(
                    Field::new()
                        .label("Bitrate (bps)")
                        .child(Input::new(&self.bitrate)),
                )
            }
            _ => {}
        }
        form = form.child(components::path_field(
            "Report (optional)",
            "browse-predict-report",
            &self.report,
            cx.listener(|this, _, window, cx| this.choose_report(window, cx)),
        ));
        let body = div().size_full().min_h_0().overflow_y_scrollbar().child(
            div()
                .p_5()
                .flex()
                .flex_col()
                .gap_5()
                .child(self.video.clone())
                .child(form),
        );
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        let ready = self.source.read(cx).valid_path().is_some();
        let footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(div().flex().justify_end().child(if active {
                Button::new("cancel-predict")
                    .danger()
                    .label(if matches!(self.stage, Stage::Cancelling) {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    })
                    .disabled(matches!(self.stage, Stage::Cancelling))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
            } else {
                Button::new("start-predict")
                    .primary()
                    .label("Predict")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
            }));
        components::inspector(body.into_any_element(), footer.into_any_element(), cx)
    }
}

fn metric(label: &'static str, value: String, muted: Hsla) -> impl IntoElement {
    div()
        .min_w_24()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(muted).child(label))
        .child(div().font_medium().child(value))
}
