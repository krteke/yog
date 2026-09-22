mod picker;
mod run;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::{
    app::App,
    form::{CommandForm, Field, Mode},
    ui::{picker::DrawPicker, run::DrawRuning},
};

const ACCENT: Color = Color::Rgb(125, 195, 255);
const INACTIVE: Color = Color::DarkGray;

pub trait DrawApp {
    fn draw(&mut self, app: &App);
    fn render_shell(&mut self, area: Rect, title: &str) -> Rect;
    fn render_modes(&mut self, area: Rect, mode: Mode);
    fn render_section(&mut self, area: Rect, title: &str, fields: &[Field], form: &CommandForm);
    fn render_footer(&mut self, area: Rect, error: Option<&str>);
}

impl DrawApp for Frame<'_> {
    fn draw(&mut self, app: &App) {
        let area = self.area();

        if let Some(state) = app.run() {
            self.draw_run(area, state);
            return;
        }

        let form = app.form();
        let content = self.render_shell(area, form.mode().label());
        let source = form.source_fields();
        let video = form.video_fields();
        let options = form.option_fields();
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(source.len() as u16 + 2),
            Constraint::Length(video.len() as u16 + 2),
            Constraint::Length(options.len() as u16 + 2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(content);

        self.render_modes(rows[0], form.mode());
        self.render_section(rows[1], " Source ", &source, form);
        self.render_section(rows[2], " Video ", &video, form);
        self.render_section(rows[3], " Options ", &options, form);
        self.render_footer(rows[5], app.error());

        if let Some(file_picker) = app.picker() {
            self.draw_picker(area, file_picker);
        }
    }

    fn render_shell(&mut self, area: Rect, title: &str) -> Rect {
        let shell = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(Line::from(format!(" {title} ")));
        let content = shell.inner(area);
        self.render_widget(shell, area);
        content
    }

    fn render_modes(&mut self, area: Rect, mode: Mode) {
        let tab = |label, selected| {
            Span::styled(
                if selected {
                    format!("[ {label} ]")
                } else {
                    format!("  {label}  ")
                },
                if selected {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(INACTIVE)
                },
            )
        };
        self.render_widget(
            Paragraph::new(Line::from(vec![
                tab("Transcode", mode == Mode::Transcode),
                Span::raw("  "),
                tab("Predict", mode == Mode::Predict),
                Span::raw("  "),
                tab("Emulate", mode == Mode::Emulate),
            ]))
            .alignment(Alignment::Center),
            area,
        );
    }

    fn render_section(&mut self, area: Rect, title: &str, fields: &[Field], form: &CommandForm) {
        let focused = form.focused();
        let active = fields.contains(&focused);
        let color = if active { ACCENT } else { INACTIVE };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .title(Span::styled(title, Style::default().fg(color)));
        let content = block.inner(area);
        self.render_widget(block, area);

        let lines = fields
            .iter()
            .copied()
            .map(|field| field_line(form, field, focused == field))
            .collect::<Vec<_>>();
        self.render_widget(Paragraph::new(lines), content);
    }

    fn render_footer(&mut self, area: Rect, error: Option<&str>) {
        if let Some(error) = error {
            self.render_widget(
                Paragraph::new(error)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::LightRed)),
                area,
            );
            return;
        }

        let line = Line::from(vec![
            key("Tab"),
            Span::raw(" Page    "),
            key("j/k"),
            Span::raw(" Move    "),
            key("h/l"),
            Span::raw(" Change    "),
            key("Enter"),
            Span::raw(" Select/Edit    "),
            key("s"),
            Span::raw(" Start    "),
            key("q"),
            Span::raw(" Quit"),
        ])
        .alignment(Alignment::Center);

        self.render_widget(Paragraph::new(line), area);
    }
}

fn field_line(form: &CommandForm, field: Field, focused: bool) -> Line<'static> {
    let marker = if focused { "›" } else { " " };
    let marker_style = Style::default().fg(if focused { ACCENT } else { INACTIVE });
    let value_style = if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let value_style = if form.is_editing(field) {
        value_style.add_modifier(Modifier::UNDERLINED)
    } else {
        value_style
    };

    if field == Field::Start {
        return Line::from(vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled("[ Start ]", value_style),
        ])
        .alignment(Alignment::Center);
    }

    Line::from(vec![
        Span::styled(format!(" {marker} "), marker_style),
        Span::styled(
            format!("{:<13}", CommandForm::label(field)),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(form.value(field), value_style),
    ])
}

fn key(value: &'static str) -> Span<'static> {
    Span::styled(
        value,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}
