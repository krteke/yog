use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap},
};
use yog_runtime::{RunStatus, event::RunPhase};

use crate::{
    run::{NoticeKind, RunKind, RunStage, RunState, format_bytes},
    ui::DrawApp,
};

use super::ACCENT;

const SPINNER: &[&str] = &["◐", "◓", "◑", "◒"];

pub trait DrawRuning {
    fn draw_run(&mut self, area: Rect, state: &RunState);
    fn render_status(&mut self, area: Rect, state: &RunState);
    fn render_progress(&mut self, area: Rect, state: &RunState);
    fn render_prediction(&mut self, area: Rect, state: &RunState);
    fn render_emulation(&mut self, area: Rect, state: &RunState);
    fn render_tasks(&mut self, area: Rect, state: &RunState);
    fn render_notices(&mut self, area: Rect, state: &RunState);
    fn render_footer_run(&mut self, area: Rect, stage: RunStage);
}

impl DrawRuning for Frame<'_> {
    fn draw_run(&mut self, area: Rect, state: &RunState) {
        let content = self.render_shell(area, state.kind.label());
        let status = match state.kind {
            RunKind::Transcode => 5,
            RunKind::Predict => 4,
            RunKind::Emulate => {
                4 + state.chart_png.iter().chain(state.chart_svg.iter()).count() as u16
            }
        };
        let details = if matches!(state.kind, RunKind::Predict | RunKind::Emulate) {
            4
        } else {
            0
        };
        let rows = Layout::vertical([
            Constraint::Length(status),
            Constraint::Length(4),
            Constraint::Length(details),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(content);

        self.render_status(rows[0], state);
        self.render_progress(rows[1], state);
        if state.kind == RunKind::Predict {
            self.render_prediction(rows[2], state);
        } else if state.kind == RunKind::Emulate {
            self.render_emulation(rows[2], state);
        }
        self.render_tasks(rows[3], state);
        self.render_notices(rows[4], state);
        self.render_footer_run(rows[5], state.stage);
    }

    fn render_status(&mut self, area: Rect, state: &RunState) {
        let color = status_color(state.stage);
        let block = section(" Status ", color);
        let content = block.inner(area);
        self.render_widget(block, area);

        let status = match state.stage {
            RunStage::Running => format!(
                "{} {}...",
                SPINNER[state.spinner % SPINNER.len()],
                state.phase.map_or("Starting", phase_label),
            ),
            RunStage::Cancelling => {
                format!("{} Cancelling...", SPINNER[state.spinner % SPINNER.len()])
            }
            RunStage::Finished(RunStatus::Success) => "✓ Completed".to_owned(),
            RunStage::Finished(RunStatus::Failure) => "× Failed".to_owned(),
            RunStage::Finished(RunStatus::Cancelled) => "■ Cancelled".to_owned(),
        };
        let input = state
            .input
            .as_deref()
            .map_or_else(|| "—".to_owned(), |path| path.display().to_string());
        let output = state
            .output
            .as_deref()
            .map_or_else(|| "—".to_owned(), |path| path.display().to_string());
        let mut lines = vec![
            Line::styled(
                status,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            path_line("Input", input),
        ];
        if state.kind == RunKind::Transcode {
            lines.push(path_line("Output", output));
        } else if state.kind == RunKind::Emulate {
            if let Some(path) = &state.chart_png {
                lines.push(path_line("PNG", path.display().to_string()));
            }
            if let Some(path) = &state.chart_svg {
                lines.push(path_line("SVG", path.display().to_string()));
            }
        }
        self.render_widget(Paragraph::new(lines), content);
    }

    fn render_progress(&mut self, area: Rect, state: &RunState) {
        let block = section(" Progress ", ACCENT);
        let content = block.inner(area);
        self.render_widget(block, area);
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(content);

        let ratio = state.progress_ratio();
        let task = match (state.kind, state.emulation.as_ref(), state.task_index) {
            (RunKind::Emulate, Some(emulation), _) => {
                format!("Point {}/{}", emulation.index, emulation.total)
            }
            (_, _, 0) => "Waiting".to_owned(),
            _ => format!("Task {}/{}", state.task_index, state.task_total),
        };
        self.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(ACCENT).bg(Color::Black))
                .ratio(ratio)
                .label(format!("{:>3}% · {task}", (ratio * 100.0).round() as u8)),
            rows[0],
        );

        let mut metrics = vec![format!("Elapsed {}", format_duration(state.elapsed()))];
        if let Some(emulation) = &state.emulation {
            metrics.push(format!("{} {}", emulation.parameter, emulation.quality));
            if let Some(candidate) = emulation.candidate {
                metrics.push(format!("Candidate #{candidate}"));
            }
        }
        if let Some(time) = state.progress.out_time_us {
            metrics.push(format!(
                "Processed {}",
                format_duration(Duration::from_micros(time.max(0) as u64))
            ));
        }
        if let Some(fps) = state.progress.fps {
            metrics.push(format!("{fps:.1} fps"));
        }
        if let Some(speed) = state.progress.speed {
            metrics.push(format!("{speed:.2}x"));
        }
        if let Some(bytes) = state.progress.total_size {
            metrics.push(format_bytes(bytes));
        }
        self.render_widget(
            Paragraph::new(metrics.join("  ·  "))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Gray)),
            rows[1],
        );
    }

    fn render_tasks(&mut self, area: Rect, state: &RunState) {
        if state.kind == RunKind::Emulate {
            let block = section(" Points ", Color::DarkGray);
            let content = block.inner(area);
            self.render_widget(block, area);
            let (succeeded, failed, remaining) =
                state.emulation.as_ref().map_or((0, 0, 0), |emulation| {
                    (
                        emulation.succeeded,
                        emulation.failed,
                        emulation.total.saturating_sub(emulation.completed),
                    )
                });
            self.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("{succeeded} succeeded"),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw("    "),
                    Span::styled(
                        format!("{failed} failed"),
                        Style::default().fg(if failed == 0 {
                            Color::DarkGray
                        } else {
                            Color::LightRed
                        }),
                    ),
                    Span::raw("    "),
                    Span::styled(
                        format!("{remaining} remaining"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
                .alignment(Alignment::Center),
                content,
            );
            return;
        }

        let block = section(" Tasks ", Color::DarkGray);
        let content = block.inner(area);
        self.render_widget(block, area);
        self.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{} succeeded", state.succeeded),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("    "),
                Span::styled(
                    format!("{} failed", state.failed),
                    Style::default().fg(if state.failed == 0 {
                        Color::DarkGray
                    } else {
                        Color::LightRed
                    }),
                ),
                Span::raw("    "),
                Span::styled(
                    format!("{} skipped", state.skipped),
                    Style::default().fg(if state.skipped == 0 {
                        Color::DarkGray
                    } else {
                        Color::Yellow
                    }),
                ),
            ]))
            .alignment(Alignment::Center),
            content,
        );
    }

    fn render_prediction(&mut self, area: Rect, state: &RunState) {
        let block = section(" Prediction ", ACCENT);
        let content = block.inner(area);
        self.render_widget(block, area);

        let Some(prediction) = &state.prediction else {
            self.render_widget(
                Paragraph::new("Waiting for prediction")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                content,
            );
            return;
        };

        let mut quality = vec![format!("VMAF {:.2}", prediction.vmaf)];
        if let Some(ssim) = prediction.ssim {
            quality.push(format!("SSIM {ssim:.4}"));
        }
        if let Some(psnr) = prediction.psnr_y_db {
            quality.push(format!("PSNR Y {psnr:.2} dB"));
        }
        let estimate = [
            format_bytes(prediction.output_bytes),
            format!(
                "{} estimated",
                format_duration(Duration::from_secs_f64(prediction.transcode_seconds))
            ),
            format!("{:.2}x", prediction.speed),
            format!("{} samples", prediction.samples),
        ]
        .join("  ·  ");
        self.render_widget(
            Paragraph::new(vec![
                Line::from(quality.join("  ·  ")).alignment(Alignment::Center),
                Line::from(estimate).alignment(Alignment::Center),
            ]),
            content,
        );
    }

    fn render_emulation(&mut self, area: Rect, state: &RunState) {
        let block = section(" Emulation ", ACCENT);
        let content = block.inner(area);
        self.render_widget(block, area);

        let Some(emulation) = &state.emulation else {
            self.render_widget(
                Paragraph::new("Waiting for first point")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                content,
            );
            return;
        };

        let label = emulation.candidate.map_or_else(
            || emulation.label.clone(),
            |candidate| format!("Candidate #{candidate} · {}", emulation.label),
        );
        let result = match (emulation.vmaf, emulation.output_bytes) {
            (Some(vmaf), Some(bytes)) => format!(
                "{} {}  ·  VMAF {vmaf:.2}  ·  {}",
                emulation.parameter,
                emulation.quality,
                format_bytes(bytes)
            ),
            _ if emulation.completed == emulation.index => {
                format!("{} {}  ·  Failed", emulation.parameter, emulation.quality)
            }
            _ => format!(
                "{} {}  ·  Predicting...",
                emulation.parameter, emulation.quality
            ),
        };
        self.render_widget(
            Paragraph::new(vec![
                Line::from(label).alignment(Alignment::Center),
                Line::from(result).alignment(Alignment::Center),
            ]),
            content,
        );
    }

    fn render_notices(&mut self, area: Rect, state: &RunState) {
        let block = section(" Events ", Color::DarkGray);
        let content = block.inner(area);
        self.render_widget(block, area);

        let visible = usize::from(content.height);
        let lines = state
            .notices
            .iter()
            .rev()
            .take(visible)
            .map(|notice| {
                let (marker, color) = match notice.kind {
                    NoticeKind::Info => ("·", Color::Gray),
                    NoticeKind::Warning => ("!", Color::Yellow),
                    NoticeKind::Error => ("×", Color::LightRed),
                };
                Line::from(vec![
                    Span::styled(format!(" {marker} "), Style::default().fg(color)),
                    Span::styled(notice.text.clone(), Style::default().fg(color)),
                ])
            })
            .collect::<Vec<_>>();
        self.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
    }

    fn render_footer_run(&mut self, area: Rect, stage: RunStage) {
        let line = match stage {
            RunStage::Running => Line::from(vec![key("q/Esc"), Span::raw(" Cancel")]),
            RunStage::Cancelling => {
                Line::styled("Cancelling...", Style::default().fg(Color::Yellow))
            }
            RunStage::Finished(_) => Line::from(vec![
                key("Enter"),
                Span::raw(" Back    "),
                key("q"),
                Span::raw(" Quit"),
            ]),
        }
        .alignment(Alignment::Center);
        self.render_widget(Paragraph::new(line), area);
    }
}

fn section(title: &'static str, color: Color) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color)))
}

fn path_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::default().fg(Color::Gray)),
        Span::raw(value),
    ])
}

fn key(value: &'static str) -> Span<'static> {
    Span::styled(
        value,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}

fn phase_label(phase: RunPhase) -> &'static str {
    match phase {
        RunPhase::Discovering => "Discovering",
        RunPhase::Probing => "Probing",
        RunPhase::Planning => "Planning",
        RunPhase::Predicting => "Predicting",
        RunPhase::Transcoding => "Transcoding",
        RunPhase::Verifying => "Verifying",
        RunPhase::Publishing => "Publishing",
        RunPhase::CalculatingVmaf => "Calculating VMAF",
        RunPhase::RenderingChart => "Rendering chart",
    }
}

fn status_color(stage: RunStage) -> Color {
    match stage {
        RunStage::Running => ACCENT,
        RunStage::Cancelling => Color::Yellow,
        RunStage::Finished(RunStatus::Success) => Color::Green,
        RunStage::Finished(RunStatus::Failure) => Color::LightRed,
        RunStage::Finished(RunStatus::Cancelled) => Color::Yellow,
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
