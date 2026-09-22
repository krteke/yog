use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::candidate::{CandidateEditor, CandidateField};

use super::ACCENT;

pub trait DrawCandidates {
    fn draw_candidates(&mut self, area: Rect, editor: &CandidateEditor);
    fn render_candidate_list(&mut self, area: Rect, editor: &CandidateEditor);
    fn render_candidate_settings(&mut self, area: Rect, editor: &CandidateEditor);
}

impl DrawCandidates for Frame<'_> {
    fn draw_candidates(&mut self, area: Rect, editor: &CandidateEditor) {
        let area = popup_area(area);
        self.render_widget(Clear, area);

        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(Span::styled(
                " Emulation candidates ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        let content = block.inner(area);
        self.render_widget(block, area);

        let list_height = editor.candidates().len().clamp(1, 4) as u16 + 2;
        let settings_height = editor.fields().len().max(1) as u16 + 2;
        let rows = Layout::vertical([
            Constraint::Length(list_height),
            Constraint::Length(settings_height),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(content);
        self.render_candidate_list(rows[0], editor);
        self.render_candidate_settings(rows[1], editor);
        let text_editing = editor
            .focused()
            .is_some_and(|field| editor.is_text_editing(field));
        let footer = if text_editing {
            Line::from(vec![
                key("←/→"),
                Span::raw(" Cursor  "),
                key("Enter/Esc"),
                Span::raw(" Done"),
            ])
        } else if editor.is_editing() {
            Line::from(vec![
                key("j/k"),
                Span::raw(" Move "),
                key("h/l"),
                Span::raw(" Change "),
                key("Enter"),
                Span::raw(" Edit/Save "),
                key("s"),
                Span::raw(" Save "),
                key("Esc"),
                Span::raw(" Cancel"),
            ])
        } else {
            Line::from(vec![
                key("j/k"),
                Span::raw(" Select "),
                key("Enter"),
                Span::raw(" Edit "),
                key("a"),
                Span::raw(" Add "),
                key("d"),
                Span::raw(" Delete "),
                key("s"),
                Span::raw(" Apply "),
                key("Esc"),
                Span::raw(" Cancel"),
            ])
        };
        self.render_widget(Paragraph::new(footer.alignment(Alignment::Center)), rows[3]);
    }

    fn render_candidate_list(&mut self, area: Rect, editor: &CandidateEditor) {
        let block = section(" Candidates ");
        let content = block.inner(area);
        self.render_widget(block, area);

        if editor.candidates().is_empty() {
            self.render_widget(
                Paragraph::new(" No candidates").style(Style::default().fg(Color::DarkGray)),
                content,
            );
            return;
        }

        let visible = usize::from(content.height);
        let max_start = editor.candidates().len().saturating_sub(visible);
        let start = editor.selected().saturating_sub(visible / 2).min(max_start);
        let lines = editor
            .candidates()
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, candidate)| {
                let selected = !editor.is_new_candidate() && editor.selected() == index;
                Line::styled(
                    format!(
                        " {} #{}  {}",
                        if selected { "›" } else { " " },
                        index + 1,
                        candidate.summary()
                    ),
                    if selected {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )
            })
            .collect::<Vec<_>>();
        self.render_widget(Paragraph::new(lines), content);
    }

    fn render_candidate_settings(&mut self, area: Rect, editor: &CandidateEditor) {
        let title = if editor.is_new_candidate() {
            " New candidate ".to_owned()
        } else if editor.is_editing() {
            format!(" Edit candidate #{} ", editor.selected() + 1)
        } else {
            " Settings ".to_owned()
        };
        let color = if editor.is_editing() {
            ACCENT
        } else {
            Color::DarkGray
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .title(Span::styled(title, Style::default().fg(color)));
        let content = block.inner(area);
        self.render_widget(block, area);

        let focused = editor.focused();
        let lines = editor
            .fields()
            .into_iter()
            .map(|field| candidate_line(editor, field, focused == Some(field)))
            .collect::<Vec<_>>();
        self.render_widget(Paragraph::new(lines), content);
    }
}

fn candidate_line(editor: &CandidateEditor, field: CandidateField, focused: bool) -> Line<'static> {
    let marker = if focused { "›" } else { " " };
    Line::from(vec![
        Span::styled(
            format!(" {marker} "),
            Style::default().fg(if focused { ACCENT } else { Color::DarkGray }),
        ),
        Span::styled(
            format!("{:<13}", field.label()),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(editor.value(field), {
            let style = if focused {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            if editor.is_text_editing(field) {
                style.add_modifier(Modifier::UNDERLINED)
            } else {
                style
            }
        }),
    ])
}

fn section(title: &'static str) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(title, Style::default().fg(Color::Gray)))
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
        Constraint::Min(56),
        Constraint::Length(4),
    ])
    .split(area);
    let vertical = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(16),
        Constraint::Length(2),
    ])
    .split(horizontal[1]);
    vertical[1]
}
