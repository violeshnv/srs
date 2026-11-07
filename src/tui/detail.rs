use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use fsrs::{FSRS, MemoryState};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, Widget},
};

use crate::data::{History, Rating, Schedule, retrievability};

use super::{
    Status, dialog::Confirm, dialog::ConfirmAction, dialog::Dialog, dialog::Outcome,
    dialog::TextInput, event::Event, fmt_due, fmt_duration, fmt_retrievability, fmt_when,
    rating_style,
};

/// The detail page owns its own state: which item it shows, the selected
/// review, and its dialog. Keeping it around between visits preserves the
/// selection when reopening the same item.
pub(crate) struct Detail {
    pub(crate) name: String,
    pub(crate) focus: usize,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) status: Option<Status>,
}

impl Detail {
    pub(crate) fn new(name: String, focus: usize) -> Self {
        Self {
            name,
            focus,
            dialog: None,
            status: None,
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent, history: &History) -> Option<Event> {
        self.status = None;

        if let Some(dialog) = &mut self.dialog {
            let outcome = dialog.handle_key_event(key);
            return self.apply_dialog(outcome?);
        }

        match key.code {
            KeyCode::Esc
            | KeyCode::Left
            | KeyCode::Char('h')
            | KeyCode::Char('H')
            | KeyCode::Char('q')
            | KeyCode::Char('Q') => Some(Event::Back),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.focus = self.focus.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                let len = history.reviews().len();
                self.focus = self.focus.saturating_add(1).min(len.saturating_sub(1));
                None
            }
            KeyCode::Char(character @ '1'..='4') => {
                if history.reviews().is_empty() {
                    self.status = Some(Status::info("This item has no reviews yet"));
                    return None;
                }
                Rating::from_char(character).map(|rating| Event::SetRating {
                    index: self.focus,
                    rating,
                })
            }
            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => {
                self.delete_review_dialog(history);
                None
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.edit_time_dialog(history);
                None
            }
            KeyCode::Char('?') => Some(Event::Help),
            _ => None,
        }
    }

    fn apply_dialog(&mut self, outcome: Outcome) -> Option<Event> {
        let dialog = self.dialog.take()?;
        if outcome == Outcome::Cancel {
            return None;
        }
        match dialog {
            Dialog::Confirm(confirm) => match confirm.action {
                ConfirmAction::RemoveReview { index, .. } => Some(Event::DeleteReview(index)),
                _ => None,
            },
            Dialog::SetTime { index, input, .. } => {
                if input.value().trim().is_empty() {
                    return None;
                }
                match parse_local_time(input.value()) {
                    Some(when) => Some(Event::EditTime { index, when }),
                    None => {
                        self.dialog = Some(Dialog::Error(
                            "Could not parse the time. Use YYYY-MM-DD HH:MM (local time)."
                                .to_string(),
                        ));
                        None
                    }
                }
            }
            Dialog::Error(_) | Dialog::Help { .. } => None,
            Dialog::Add(_) | Dialog::Rename { .. } => None,
        }
    }

    fn delete_review_dialog(&mut self, history: &History) {
        let Some(review) = history.reviews().get(self.focus).copied() else {
            self.status = Some(Status::info("This item has no reviews yet"));
            return;
        };
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Delete review".to_string(),
            message: format!(
                "Delete review #{} ({}, {})?\nLater reviews will be replayed.",
                self.focus + 1,
                review.rating,
                fmt_when(review.when)
            ),
            action: ConfirmAction::RemoveReview { index: self.focus },
        }));
    }

    fn edit_time_dialog(&mut self, history: &History) {
        let Some(review) = history.reviews().get(self.focus).copied() else {
            self.status = Some(Status::info("This item has no reviews yet"));
            return;
        };
        let local = review
            .when
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        self.dialog = Some(Dialog::SetTime {
            index: self.focus,
            input: TextInput::new(local),
        });
    }
}

pub(crate) fn render(
    frame: &mut Frame,
    detail: &Detail,
    history: &History,
    schedule: Option<&Schedule>,
    fsrs: &FSRS,
    desired_retention: f32,
) {
    let area = frame.area();
    let now = Utc::now();

    let instructions = Line::from(vec![
        " Back ".into(),
        "<ESC>".blue().bold(),
        " Change rating ".into(),
        "<1-4>".blue().bold(),
        " Delete review ".into(),
        "<X>".blue().bold(),
        " Edit time ".into(),
        "<T>".blue().bold(),
        " Help ".into(),
        "<?>".blue().bold(),
        " ".into(),
    ]);

    let mut block = Block::bordered()
        .title(
            Line::from(format!(" Detail — {} ", detail.name))
                .bold()
                .centered(),
        )
        .title_bottom(instructions.centered())
        .border_set(border::THICK);
    if let Some(status) = &detail.status {
        block = block.title_top(
            Line::from(format!(" {} ", status.text))
                .right_aligned()
                .style(status.style()),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [summary_area, table_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).areas(inner);

    frame.render_widget(
        Paragraph::new(Text::from(summary_lines(
            history,
            schedule,
            fsrs,
            desired_retention,
            now,
        ))),
        summary_area,
    );

    let rows: Vec<Row> = schedule
        .map(|schedule| {
            schedule
                .reviews()
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
                    if index == detail.focus {
                        row.style(Style::new().reversed().bold())
                    } else {
                        row
                    }
                })
                .collect()
        })
        .unwrap_or_default();

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

    if let Some(dialog) = &detail.dialog {
        dialog.render(frame.area(), frame.buffer_mut());
    }
}

fn summary_lines(
    history: &History,
    schedule: Option<&Schedule>,
    fsrs: &FSRS,
    desired_retention: f32,
    now: DateTime<Utc>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if history.reviews().is_empty() {
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
    } else if let Some(schedule) = schedule {
        let memory = schedule.memory();
        let last_review = schedule.last_review();
        let retrievability = memory
            .zip(last_review)
            .map(|(memory, last_review)| retrievability(memory, last_review, now));

        let first = vec![
            "  Reviews ".into(),
            history.reviews().len().to_string().bold(),
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
                .map(fmt_retrievability)
                .unwrap_or_else(|| "-".to_string())
                .into(),
        ];
        lines.push(Line::from(first));

        lines.push(Line::from(format!(
            "  Due {} ({})   Interval {}",
            fmt_when(schedule.due().expect("non-empty schedule")),
            fmt_due(schedule.due().expect("non-empty schedule"), now),
            fmt_duration(schedule.interval_days().expect("non-empty schedule"))
        )));
    } else {
        lines.push(
            Line::from("  Could not replay this history.").style(Style::new().fg(Color::Red)),
        );
    }

    lines.push(Line::from(next_states_text(
        fsrs,
        desired_retention,
        schedule.and_then(Schedule::memory),
        schedule.and_then(Schedule::last_review),
        now,
    )));

    lines
}

/// Intervals for the four answer buttons if the item were reviewed right now,
/// computed with the high-level `FSRS::next_states` API.
fn next_states_text(
    fsrs: &FSRS,
    desired_retention: f32,
    memory: Option<MemoryState>,
    last_review: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> String {
    let (state, elapsed_days) = match (memory, last_review) {
        (Some(memory), Some(last_review)) => {
            let elapsed = (now - last_review).num_seconds().max(0) / 86_400;
            (Some(memory), elapsed.min(u32::MAX as i64) as u32)
        }
        _ => (None, 0),
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

/// Parse a local timestamp written as `YYYY-MM-DD HH:MM`.
fn parse_local_time(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    for format in [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|time| time.with_timezone(&Utc));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0)?;
        return Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|time| time.with_timezone(&Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_times() {
        assert!(parse_local_time("2026-11-05 12:30").is_some());
        assert!(parse_local_time("2026-11-05").is_some());
        assert!(parse_local_time("  ").is_none());
        assert!(parse_local_time("not a date").is_none());
    }
}
