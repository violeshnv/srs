use chrono::Utc;
use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, Widget},
};

use super::{app::App, fmt_due, fmt_duration, fmt_retrievability, retrievability_style};

pub(crate) fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let now = Utc::now();
    let rows_data = app.ordered_rows();

    let file_name = app
        .manager
        .path()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.manager.path().display().to_string());

    let instructions = Line::from(vec![
        " Open ".into(),
        "<Enter>".blue().bold(),
        " Rate ".into(),
        "<1-4>".blue().bold(),
        " Add ".into(),
        "<a>".blue().bold(),
        " Rename ".into(),
        "<e>".blue().bold(),
        " Delete ".into(),
        "<d>".blue().bold(),
        " Undo ".into(),
        "<u>".blue().bold(),
        " Sort ".into(),
        "<s>".blue().bold(),
        " Help ".into(),
        "<?>".blue().bold(),
        " Quit ".into(),
        "<q>".blue().bold(),
    ]);

    let mut block = Block::bordered()
        .title(Line::from(format!(" SRS — {file_name} ")).bold().centered())
        .title_top(Line::from(format!(" sort: {} ", app.sort.label())).left_aligned())
        .title_bottom(instructions.centered())
        .border_set(border::THICK);
    if let Some(status) = &app.status {
        block = block.title_top(
            Line::from(format!(" {} ", status.text))
                .right_aligned()
                .style(status.style()),
        );
    }

    if rows_data.is_empty() {
        let empty = Text::from(vec![
            Line::from(""),
            Line::from("  No items yet.").bold(),
            Line::from("  Press <a> to add your first item."),
        ]);
        frame.render_widget(Paragraph::new(empty).block(block), area);
        return;
    }

    let header = Row::new([
        "Name",
        "Due",
        "Interval",
        "Reps",
        "Stability",
        "Difficulty",
        "Retrievability",
    ])
    .style(Style::new().bold());

    let widths = [
        Constraint::Percentage(26),
        Constraint::Percentage(17),
        Constraint::Percentage(12),
        Constraint::Percentage(8),
        Constraint::Percentage(14),
        Constraint::Percentage(12),
        Constraint::Percentage(11),
    ];

    let rows = rows_data
        .iter()
        .enumerate()
        .map(|(index, (history, summary))| {
            let (due, due_style) = match summary {
                Some(summary) if summary.reviews == 0 => ("new".to_string(), Style::new()),
                Some(summary) => (
                    fmt_due(summary.due, now),
                    if summary.due < now {
                        Style::new().fg(Color::Red)
                    } else {
                        Style::new()
                    },
                ),
                None => ("?".to_string(), Style::new().fg(Color::Red)),
            };

            let interval = match summary {
                Some(summary) if summary.reviews > 0 => fmt_duration(summary.interval_days),
                _ => "-".to_string(),
            };
            let stability = match summary.and_then(|summary| summary.memory) {
                Some(memory) => fmt_duration(memory.stability),
                None => "-".to_string(),
            };
            let difficulty = match summary.and_then(|summary| summary.memory) {
                Some(memory) => format!("{:.1}", memory.difficulty),
                None => "-".to_string(),
            };
            let (retrievability, retrievability_style) =
                match summary.and_then(|summary| summary.retrievability(now)) {
                    Some(value) => (fmt_retrievability(value), retrievability_style(value)),
                    None => ("-".to_string(), Style::new()),
                };

            let cells = [
                Cell::from(history.name().to_string()),
                Cell::from(due).style(due_style),
                Cell::from(interval),
                Cell::from(history.reviews().len().to_string()),
                Cell::from(stability),
                Cell::from(difficulty),
                Cell::from(retrievability).style(retrievability_style),
            ];

            let row = Row::new(cells);
            if index == app.focus {
                row.style(Style::new().reversed().bold())
            } else {
                row
            }
        });

    Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(block)
        .render(area, frame.buffer_mut());
}
