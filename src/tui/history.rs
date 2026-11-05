use chrono::Utc;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, Widget},
};

use crate::data::{History, Schedule, Summary};

use super::{app::App, fmt_due, fmt_duration, fmt_when, rating_style};

pub(crate) fn render(app: &App, name: &str, focus: usize, frame: &mut Frame) {
    let area = frame.area();
    let now = Utc::now();

    let Some(history) = app.manager.get(name) else {
        frame.render_widget(
            Paragraph::new(format!("Item \"{name}\" no longer exists. Press <Esc>.")),
            area,
        );
        return;
    };

    let instructions = Line::from(vec![
        " Back ".into(),
        "<Esc>".blue().bold(),
        " Change rating ".into(),
        "<1-4>".blue().bold(),
        " Delete review ".into(),
        "<x>".blue().bold(),
        " Edit time ".into(),
        "<t>".blue().bold(),
        " Help ".into(),
        "<?>".blue().bold(),
    ]);

    let mut block = Block::bordered()
        .title(Line::from(format!(" History — {name} ")).bold().centered())
        .title_bottom(instructions.centered())
        .border_set(border::THICK);
    if let Some(status) = &app.status {
        block = block.title_top(
            Line::from(format!(" {} ", status.text))
                .right_aligned()
                .style(status.style()),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let schedule = history.schedule(app.manager.fsrs(), app.manager.desired_retention());
    let summary = history
        .summary(app.manager.fsrs(), app.manager.desired_retention())
        .ok();

    let [summary_area, table_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).areas(inner);

    frame.render_widget(
        Paragraph::new(Text::from(summary_lines(
            app, history, &schedule, summary, now,
        ))),
        summary_area,
    );

    let rows: Vec<Row> = match &schedule {
        Ok(schedule) => schedule
            .reviews
            .iter()
            .enumerate()
            .map(|(index, scheduled)| {
                let cells = [
                    Cell::from((index + 1).to_string()),
                    Cell::from(fmt_when(scheduled.review.when)),
                    Cell::from(scheduled.review.rating.label())
                        .style(rating_style(scheduled.review.rating)),
                    Cell::from(if index == 0 {
                        "-".to_string()
                    } else {
                        fmt_duration(scheduled.elapsed_days)
                    }),
                    Cell::from(fmt_duration(scheduled.memory.stability)),
                    Cell::from(format!("{:.1}", scheduled.memory.difficulty)),
                    Cell::from(fmt_duration(scheduled.interval_days)),
                    Cell::from(fmt_when(scheduled.due)),
                ];
                let row = Row::new(cells);
                if index == focus {
                    row.style(Style::new().reversed().bold())
                } else {
                    row
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let header = Row::new([
        "#",
        "When",
        "Rating",
        "Elapsed",
        "Stability",
        "Difficulty",
        "Interval",
        "Due",
    ])
    .style(Style::new().bold());

    let widths = [
        Constraint::Percentage(5),
        Constraint::Percentage(20),
        Constraint::Percentage(10),
        Constraint::Percentage(9),
        Constraint::Percentage(12),
        Constraint::Percentage(11),
        Constraint::Percentage(11),
        Constraint::Percentage(22),
    ];

    Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .render(table_area, frame.buffer_mut());
}

fn summary_lines(
    app: &App,
    history: &History,
    schedule: &Result<Schedule, fsrs::FSRSError>,
    summary: Option<Summary>,
    now: chrono::DateTime<Utc>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    match summary {
        Some(summary) if summary.reviews > 0 => {
            let memory = summary.memory;
            let retrievability = summary.retrievability(now);
            let mut first = vec![
                "Reviews ".into(),
                summary.reviews.to_string().bold(),
                "   Stability ".into(),
                memory
                    .map(|memory| fmt_duration(memory.stability))
                    .unwrap_or_else(|| "-".to_string())
                    .into(),
                "   Difficulty ".into(),
                memory
                    .map(|memory| format!("{:.2}", memory.difficulty))
                    .unwrap_or_else(|| "-".to_string())
                    .into(),
                "   Retrievability ".into(),
                retrievability
                    .map(super::fmt_retrievability)
                    .unwrap_or_else(|| "-".to_string())
                    .into(),
            ];
            first.insert(0, "  ".into());
            lines.push(Line::from(first));

            lines.push(Line::from(format!(
                "  Due {} ({})   Interval {}",
                fmt_when(summary.due),
                fmt_due(summary.due, now),
                fmt_duration(summary.interval_days)
            )));
        }
        _ => {
            lines.push(Line::from(vec![
                "  ".into(),
                "New item".bold(),
                format!(
                    " — no reviews yet. Created {}.",
                    fmt_when(history.created())
                )
                .into(),
            ]));
            lines.push(Line::from("  Rate it from the Home page with <1>-<4>."));
        }
    }

    match schedule {
        Ok(_) => lines.push(Line::from(next_states_text(app, summary, now))),
        Err(error) => lines.push(
            Line::from(format!("  Could not replay this history: {error}"))
                .style(Style::new().fg(Color::Red)),
        ),
    }

    lines
}

/// Intervals for the four answer buttons if the item were reviewed right now,
/// computed with the high-level `FSRS::next_states` API.
fn next_states_text(app: &App, summary: Option<Summary>, now: chrono::DateTime<Utc>) -> String {
    let fsrs = app.manager.fsrs();
    let desired_retention = app.manager.desired_retention();
    let (state, elapsed_days) = match summary {
        Some(summary) => match (summary.memory, summary.last_review) {
            (Some(memory), Some(last_review)) => {
                let elapsed = (now - last_review).num_seconds().max(0) / 86_400;
                (Some(memory), elapsed.min(u32::MAX as i64) as u32)
            }
            _ => (None, 0),
        },
        None => return "  If reviewed now — unavailable".to_string(),
    };

    match fsrs.next_states(state, desired_retention, elapsed_days) {
        Ok(states) => format!(
            "  If reviewed now — Again {}   Hard {}   Good {}   Easy {}",
            fmt_duration(states.again.interval),
            fmt_duration(states.hard.interval),
            fmt_duration(states.good.interval),
            fmt_duration(states.easy.interval),
        ),
        Err(_) => "  If reviewed now — unavailable".to_string(),
    }
}
