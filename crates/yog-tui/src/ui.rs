use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::app::{App, Field};

const ACCENT: Color = Color::Rgb(125, 195, 255);
const INACTIVE: Color = Color::DarkGray;

pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 64 || area.height < 22 {
        render_too_small(frame, area);
        return;
    }

    let shell = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " yog ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(" Transcode ").right_aligned());
    let content = shell.inner(area);
    frame.render_widget(shell, area);

    let source = app.source_fields();
    let video = app.video_fields();
    let options = app.option_fields();
    let rows = Layout::vertical([
        Constraint::Length(source.len() as u16 + 2),
        Constraint::Length(video.len() as u16 + 2),
        Constraint::Length(options.len() as u16 + 2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(content);

    render_section(frame, rows[0], " Source ", source, app);
    render_section(frame, rows[1], " Video ", &video, app);
    render_section(frame, rows[2], " Options ", &options, app);
    render_footer(frame, rows[4]);
}

fn render_section(frame: &mut Frame<'_>, area: Rect, title: &str, fields: &[Field], app: &App) {
    let focused = app.focused();
    let active = fields.contains(&focused);
    let color = if active { ACCENT } else { INACTIVE };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color)));
    let content = block.inner(area);
    frame.render_widget(block, area);

    let lines = fields
        .iter()
        .copied()
        .map(|field| field_line(app, field, focused == field))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), content);
}

fn field_line(app: &App, field: Field, focused: bool) -> Line<'static> {
    let marker = if focused { "›" } else { " " };
    let marker_style = Style::default().fg(if focused { ACCENT } else { INACTIVE });
    let value_style = if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let value_style = if app.is_editing(field) {
        value_style.add_modifier(Modifier::UNDERLINED)
    } else {
        value_style
    };

    Line::from(vec![
        Span::styled(format!(" {marker} "), marker_style),
        Span::styled(
            format!("{:<13}", App::label(field)),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(app.value(field), value_style),
    ])
}

fn render_footer(frame: &mut Frame<'_>, area: Rect) {
    let line = Line::from(vec![
        key("j/k"),
        Span::raw(" Move    "),
        key("h/l"),
        Span::raw(" Change    "),
        key("Enter"),
        Span::raw(" Edit    "),
        key("q"),
        Span::raw(" Quit"),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(Paragraph::new(line), area);
}

fn key(value: &'static str) -> Span<'static> {
    Span::styled(
        value,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " yog ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let content = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new("Terminal too small")
            .alignment(Alignment::Center)
            .style(Style::default().fg(ACCENT)),
        content,
    );
}
