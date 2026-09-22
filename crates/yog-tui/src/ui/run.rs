use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap},
};
use yog_runtime::{RunStatus, event::RunPhase};

use crate::run::{NoticeKind, RunStage, RunState, format_bytes};

use super::{ACCENT, render_shell};

const SPINNER: &[&str] = &["◐", "◓", "◑", "◒"];

pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &RunState) {
    let content = render_shell(frame, area, "Transcode");
    let rows = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(content);

    render_status(frame, rows[0], state);
    render_progress(frame, rows[1], state);
    render_tasks(frame, rows[2], state);
    render_notices(frame, rows[3], state);
    render_footer(frame, rows[4], state.stage);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, state: &RunState) {
    let color = status_color(state.stage);
    let block = section(" Status ", color);
    let content = block.inner(area);
    frame.render_widget(block, area);

    let status = match state.stage {
        RunStage::Running => format!(
            "{} {}...",
            SPINNER[state.spinner % SPINNER.len()],
            state.phase.map_or("Starting", phase_label),
        ),
        RunStage::Cancelling => format!("{} Cancelling...", SPINNER[state.spinner % SPINNER.len()]),
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
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                status,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            path_line("Input", input),
            path_line("Output", output),
        ]),
        content,
    );
}

fn render_progress(frame: &mut Frame<'_>, area: Rect, state: &RunState) {
    let block = section(" Progress ", ACCENT);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(content);

    let ratio = state.progress_ratio();
    let task = if state.task_index == 0 {
        "Waiting".to_owned()
    } else {
        format!("Task {}/{}", state.task_index, state.task_total)
    };
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(ACCENT).bg(Color::Black))
            .ratio(ratio)
            .label(format!("{:>3}% · {task}", (ratio * 100.0).round() as u8)),
        rows[0],
    );

    let mut metrics = vec![format!("Elapsed {}", format_duration(state.elapsed()))];
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
    frame.render_widget(
        Paragraph::new(metrics.join("  ·  "))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Gray)),
        rows[1],
    );
}

fn render_tasks(frame: &mut Frame<'_>, area: Rect, state: &RunState) {
    let block = section(" Tasks ", Color::DarkGray);
    let content = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
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

fn render_notices(frame: &mut Frame<'_>, area: Rect, state: &RunState) {
    let block = section(" Events ", Color::DarkGray);
    let content = block.inner(area);
    frame.render_widget(block, area);

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
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, stage: RunStage) {
    let line = match stage {
        RunStage::Running => Line::from(vec![key("q/Esc"), Span::raw(" Cancel")]),
        RunStage::Cancelling => Line::styled("Cancelling...", Style::default().fg(Color::Yellow)),
        RunStage::Finished(_) => Line::from(vec![
            key("Enter"),
            Span::raw(" Back    "),
            key("q"),
            Span::raw(" Quit"),
        ]),
    }
    .alignment(Alignment::Center);
    frame.render_widget(Paragraph::new(line), area);
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
