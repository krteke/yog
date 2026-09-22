use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::file_picker::FilePicker;

const ACCENT: Color = Color::Rgb(125, 195, 255);

pub fn draw(frame: &mut Frame<'_>, area: Rect, picker: &FilePicker) {
    let area = popup_area(area);
    frame.render_widget(Clear, area);

    let position = if picker.entries().is_empty() {
        " 0/0 ".to_owned()
    } else {
        format!(" {}/{} ", picker.cursor() + 1, picker.entries().len())
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " Select input ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(position).right_aligned());
    let content = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(content);
    frame.render_widget(
        Paragraph::new(format!(" {}", picker.directory().display()))
            .style(Style::default().fg(Color::Gray)),
        rows[0],
    );
    render_entries(frame, rows[1], picker);
    render_footer(frame, rows[2], picker.error());
}

fn render_entries(frame: &mut Frame<'_>, area: Rect, picker: &FilePicker) {
    let visible = usize::from(area.height);
    if visible == 0 {
        return;
    }
    if picker.entries().is_empty() {
        frame.render_widget(
            Paragraph::new(" Empty").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let max_start = picker.entries().len().saturating_sub(visible);
    let start = picker.cursor().saturating_sub(visible / 2).min(max_start);
    let lines = picker
        .entries()
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, entry)| {
            let selected = picker.selected() == Some(index);
            let marker = if selected { "x" } else { " " };
            let suffix = if entry.is_directory() { "/" } else { "" };
            let style = if picker.cursor() == index {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(Color::White)
            };
            Line::styled(
                format!(" [{marker}] {}{suffix}", entry.name().to_string_lossy()),
                style,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, error: Option<&str>) {
    let line = if let Some(error) = error {
        Line::styled(error.to_owned(), Style::default().fg(Color::LightRed))
    } else {
        Line::from(vec![
            key("j/k"),
            Span::raw(" Move   "),
            key("h/l"),
            Span::raw(" Parent/Open   "),
            key("Space"),
            Span::raw(" Select   "),
            key("Enter"),
            Span::raw(" Confirm   "),
            key("Esc"),
            Span::raw(" Cancel"),
        ])
    }
    .alignment(Alignment::Center);
    frame.render_widget(Paragraph::new(line), area);
}

fn key(value: &'static str) -> Span<'static> {
    Span::styled(
        value,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}

fn popup_area(area: Rect) -> Rect {
    let horizontal = Layout::horizontal([
        Constraint::Length(4),
        Constraint::Min(48),
        Constraint::Length(4),
    ])
    .split(area);
    let vertical = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(12),
        Constraint::Length(2),
    ])
    .split(horizontal[1]);
    vertical[1]
}
