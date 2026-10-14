use super::ACCENT;
use crate::recovery::RecoveryDialog;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use yog_runtime::recovery::RecoveryAction;

pub trait DrawRecovery {
    fn draw_recovery(&mut self, area: Rect, dialog: &RecoveryDialog);
}

impl DrawRecovery for Frame<'_> {
    fn draw_recovery(&mut self, area: Rect, dialog: &RecoveryDialog) {
        let width = area.width.saturating_sub(4).min(100);
        let height = area.height.saturating_sub(2).min(32);
        let area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        self.render_widget(Clear, area);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(format!(" {} ", dialog.request.title()));
        let content = block.inner(area);
        self.render_widget(block, area);
        let rows = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(content);
        let mut details = vec![
            Line::from(dialog.request.input.display().to_string()),
            Line::from(format!("State: {}", dialog.request.state_path.display())),
            Line::from(""),
        ];
        if let Some(output) = &dialog.request.output {
            details.insert(
                1,
                Line::from(format!("Recorded output: {}", output.display())),
            );
        }
        for difference in &dialog.request.differences {
            details.push(Line::styled(
                difference.kind.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            details.push(Line::from(format!("Recorded: {}", difference.recorded)));
            details.push(Line::from(format!("Current:  {}", difference.current)));
        }
        self.render_widget(
            Paragraph::new(details)
                .wrap(Wrap { trim: false })
                .scroll((dialog.scroll, 0)),
            rows[0],
        );
        let actions = RecoveryAction::ALL
            .into_iter()
            .enumerate()
            .map(|(index, action)| {
                Line::styled(
                    format!(
                        "{} {}",
                        if index == dialog.selected { ">" } else { " " },
                        dialog.request.action_label(action)
                    ),
                    if index == dialog.selected {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        self.render_widget(Paragraph::new(actions), rows[1]);
        self.render_widget(
            Paragraph::new(
                dialog
                    .request
                    .action_detail(RecoveryAction::ALL[dialog.selected]),
            )
            .wrap(Wrap { trim: false }),
            rows[2],
        );
        self.render_widget(
            Paragraph::new(format!(
                "[{}] Apply to later differences of the same kind in this run",
                if dialog.apply_to_similar { "x" } else { " " }
            ))
            .wrap(Wrap { trim: false }),
            rows[3],
        );
        self.render_widget(Paragraph::new(Line::from(vec![Span::styled("j/k", Style::default().fg(ACCENT)), Span::raw(" Select · Space Apply to similar · Enter Decide · Esc Hide · q Cancel · PgUp/PgDn Details")])), rows[4]);
    }
}
