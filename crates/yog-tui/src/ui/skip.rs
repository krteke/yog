use super::ACCENT;
use crate::skip::SkipDialog;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

pub trait DrawSkip {
    fn draw_skip(&mut self, area: Rect, dialog: &SkipDialog);
}

impl DrawSkip for Frame<'_> {
    fn draw_skip(&mut self, area: Rect, dialog: &SkipDialog) {
        let width = area.width.saturating_sub(4).min(80);
        let height = area.height.saturating_sub(2).min(12);
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
            .title(" Skip tasks ");
        let inner = block.inner(area);
        self.render_widget(block, area);
        let mut lines = vec![
            Line::from(dialog.input.display().to_string()),
            Line::from(""),
            Line::from(format!("Count: [{}]", dialog.value)),
            Line::from("Includes the current task; already completed tasks are not counted."),
        ];
        if dialog.vmaf {
            lines.push(Line::from(
                "Current video is complete; only its VMAF calculation will be skipped.",
            ));
        }
        if dialog.finished {
            lines.push(Line::from(
                "This task has finished. Close this dialog and select the current task.",
            ));
        } else if dialog.value.parse::<std::num::NonZeroUsize>().is_err() {
            lines.push(Line::from("Enter a positive whole number."));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "j/k Adjust · Enter Skip · Esc Close · Ctrl+C Cancel batch",
        ));
        self.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}
