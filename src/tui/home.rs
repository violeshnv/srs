use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use fsrs::MemoryState;
use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, Widget},
};

use crate::data::{History, Rating, Schedule, retrievability};

use super::{
    Status, dialog::Confirm, dialog::ConfirmAction, dialog::Dialog, dialog::Outcome,
    dialog::TextInput, event::Event, fmt_due, fmt_duration, fmt_retrievability, fmt_when,
    rating_style, retrievability_style,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Sort {
    #[default]
    DueAsc,
    DueDesc,
    NameAsc,
    NameDesc,
}

impl Sort {
    fn next(self) -> Self {
        match self {
            Self::DueAsc => Self::DueDesc,
            Self::DueDesc => Self::NameAsc,
            Self::NameAsc => Self::NameDesc,
            Self::NameDesc => Self::DueAsc,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::DueAsc => "due ↑",
            Self::DueDesc => "due ↓",
            Self::NameAsc => "name ↑",
            Self::NameDesc => "name ↓",
        }
    }
}

/// Read-only view of one home-page row. It owns nothing and only forwards to a
/// [`History`] (the persisted truth) and its replayed [`Schedule`].
#[derive(Clone, Copy)]
pub(crate) struct Summary<'a> {
    history: &'a History,
    schedule: Option<&'a Schedule>,
}

impl<'a> Summary<'a> {
    pub(crate) fn new(history: &'a History, schedule: Option<&'a Schedule>) -> Self {
        Self { history, schedule }
    }

    pub(crate) fn name(&self) -> &str {
        self.history.name()
    }

    pub(crate) fn reviews(&self) -> usize {
        self.history.reviews().len()
    }

    pub(crate) fn last_rating(&self) -> Option<Rating> {
        self.history.last_review().map(|review| review.rating)
    }

    pub(crate) fn last_review(&self) -> Option<DateTime<Utc>> {
        self.schedule.and_then(Schedule::last_review)
    }

    pub(crate) fn has_schedule(&self) -> bool {
        self.schedule.is_some()
    }

    pub(crate) fn memory(&self) -> Option<MemoryState> {
        self.schedule.and_then(Schedule::memory)
    }

    pub(crate) fn interval_days(&self) -> f32 {
        self.schedule
            .and_then(Schedule::interval_days)
            .unwrap_or(0.0)
    }

    pub(crate) fn due(&self) -> DateTime<Utc> {
        self.schedule
            .and_then(Schedule::due)
            .unwrap_or_else(|| self.history.created())
    }

    pub(crate) fn retrievability(&self, now: DateTime<Utc>) -> Option<f32> {
        let memory = self.memory()?;
        let last_review = self.last_review()?;
        Some(retrievability(memory, last_review, now))
    }
}

/// The home page owns its own state: sort order, selection, and dialog.
#[derive(Default)]
pub(crate) struct Home {
    sort: Sort,
    /// Name of the item under the cursor. Names are the stable identity, so the
    /// cursor stays on the same item when sorting or a review changes the order.
    focused: Option<String>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) status: Option<Status>,
}

impl Home {
    /// Order rows according to the current sort setting.
    pub(crate) fn order<'a>(&self, rows: &[Summary<'a>]) -> Vec<Summary<'a>> {
        let mut ordered = rows.to_vec();
        ordered.sort_by(|a, b| self.compare(a, b));
        ordered
    }

    fn compare(&self, a: &Summary<'_>, b: &Summary<'_>) -> Ordering {
        match self.sort {
            Sort::DueAsc => a
                .due()
                .cmp(&b.due())
                .then_with(|| natural_cmp(a.name(), b.name())),
            Sort::DueDesc => b
                .due()
                .cmp(&a.due())
                .then_with(|| natural_cmp(a.name(), b.name())),
            Sort::NameAsc => natural_cmp(a.name(), b.name()).then_with(|| a.name().cmp(b.name())),
            Sort::NameDesc => natural_cmp(b.name(), a.name()).then_with(|| b.name().cmp(a.name())),
        }
    }

    pub(crate) fn focused_index(&self, ordered: &[Summary<'_>]) -> usize {
        if ordered.is_empty() {
            return 0;
        }
        self.focused
            .as_deref()
            .and_then(|name| ordered.iter().position(|summary| summary.name() == name))
            .unwrap_or(0)
            .min(ordered.len() - 1)
    }

    fn focused_name(&self, ordered: &[Summary<'_>]) -> Option<String> {
        if ordered.is_empty() {
            None
        } else {
            Some(ordered[self.focused_index(ordered)].name().to_string())
        }
    }

    fn move_focus(&mut self, delta: isize, ordered: &[Summary<'_>]) {
        if ordered.is_empty() {
            self.focused = None;
            return;
        }
        let current = self.focused_index(ordered);
        let next = current.saturating_add_signed(delta).min(ordered.len() - 1);
        self.focused = Some(ordered[next].name().to_string());
    }

    pub(crate) fn focus_on(&mut self, name: &str) {
        self.focused = Some(name.to_string());
    }

    pub(crate) fn clear_focus(&mut self) {
        self.focused = None;
    }

    #[cfg(test)]
    pub(crate) fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
    }

    /// Returns an [`Event`] for the `App` to apply, or `None` if the key was
    /// fully handled by this page (focus movement, dialog typing, ...).
    pub(crate) fn handle_key(&mut self, key: KeyEvent, ordered: &[Summary<'_>]) -> Option<Event> {
        self.status = None;

        if let Some(dialog) = &mut self.dialog {
            let outcome = dialog.handle_key_event(key);
            return self.apply_dialog(outcome?);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Some(Event::Quit),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.move_focus(-1, ordered);
                None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                self.move_focus(1, ordered);
                None
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('L') => {
                self.focused_name(ordered).map(Event::Open)
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.dialog = Some(Dialog::Add(TextInput::default()));
                None
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.rename_dialog(ordered);
                None
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.delete_dialog(ordered);
                None
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                self.undo_dialog(ordered);
                None
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.sort = self.sort.next();
                None
            }
            KeyCode::Char('?') => Some(Event::Help),
            KeyCode::Char(character @ '1'..='4') => {
                match (Rating::from_char(character), self.focused_name(ordered)) {
                    (Some(rating), Some(name)) => Some(Event::Review { name, rating }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn apply_dialog(&mut self, outcome: Outcome) -> Option<Event> {
        let dialog = self.dialog.take()?;
        if outcome == Outcome::Cancel {
            return None;
        }
        match dialog {
            Dialog::Add(input) => Some(Event::Add(input.value().trim().to_string())),
            Dialog::Rename { old, input } => Some(Event::Rename {
                old,
                new: input.value().trim().to_string(),
            }),
            Dialog::Confirm(confirm) => match confirm.action {
                ConfirmAction::RemoveHistory(name) => Some(Event::Delete(name)),
                ConfirmAction::UndoLast(name) => Some(Event::Undo(name)),
                ConfirmAction::RemoveReview { .. } => None,
            },
            Dialog::Error(_) | Dialog::Help { .. } | Dialog::SetTime { .. } => None,
        }
    }

    fn rename_dialog(&mut self, ordered: &[Summary<'_>]) {
        if let Some(name) = self.focused_name(ordered) {
            self.dialog = Some(Dialog::Rename {
                old: name,
                input: TextInput::default(),
            });
        }
    }

    fn delete_dialog(&mut self, ordered: &[Summary<'_>]) {
        let Some(name) = self.focused_name(ordered) else {
            return;
        };
        let reviews = ordered[self.focused_index(ordered)].reviews();
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Delete item".to_string(),
            message: format!(
                "Delete \"{name}\" and its {reviews} review(s)?\nThis cannot be undone."
            ),
            action: ConfirmAction::RemoveHistory(name),
        }));
    }

    fn undo_dialog(&mut self, ordered: &[Summary<'_>]) {
        let Some(name) = self.focused_name(ordered) else {
            return;
        };
        let summary = &ordered[self.focused_index(ordered)];
        match (summary.last_rating(), summary.last_review()) {
            (Some(rating), Some(when)) => {
                self.dialog = Some(Dialog::Confirm(Confirm {
                    title: "Undo last review".to_string(),
                    message: format!(
                        "Remove the last review of \"{name}\" ({rating} at {})?",
                        fmt_when(when)
                    ),
                    action: ConfirmAction::UndoLast(name),
                }));
            }
            _ => self.status = Some(Status::info("Nothing to undo for this item")),
        }
    }
}

pub(crate) fn render(frame: &mut Frame, home: &Home, ordered: &[Summary<'_>], file_name: &str) {
    let area = frame.area();
    let now = Utc::now();
    let focus = home.focused_index(ordered);

    let instructions = Line::from(vec![
        " Open ".into(),
        "<ENTER>".blue().bold(),
        " Rate ".into(),
        "<1-4>".blue().bold(),
        " Add ".into(),
        "<A>".blue().bold(),
        " Rename ".into(),
        "<R>".blue().bold(),
        " Delete ".into(),
        "<D>".blue().bold(),
        " Undo ".into(),
        "<U>".blue().bold(),
        " Sort ".into(),
        "<S>".blue().bold(),
        " Help ".into(),
        "<?>".blue().bold(),
        " Quit ".into(),
        "<Q>".blue().bold(),
        " ".into(),
    ]);

    let mut block = Block::bordered()
        .title(Line::from(format!(" SRS — {file_name} ")).bold().centered())
        .title_top(Line::from(format!(" sort: {} ", home.sort.label())).left_aligned())
        .title_bottom(instructions.centered())
        .border_set(border::THICK);
    if let Some(status) = &home.status {
        block = block.title_top(
            Line::from(format!(" {} ", status.text))
                .right_aligned()
                .style(status.style()),
        );
    }

    if ordered.is_empty() {
        let empty = Text::from(vec![
            Line::from(""),
            Line::from("  No items yet.").bold(),
            Line::from("  Press <A> to add your first item."),
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
        "Last",
        "Retrievability",
    ])
    .style(Style::new().bold());

    let widths = [
        Constraint::Percentage(24),
        Constraint::Percentage(16),
        Constraint::Percentage(11),
        Constraint::Percentage(6),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
        Constraint::Percentage(8),
        Constraint::Percentage(13),
    ];

    let rows = ordered.iter().enumerate().map(|(index, summary)| {
        let (due, due_style) = if !summary.has_schedule() {
            ("?".to_string(), Style::new().fg(Color::Red))
        } else if summary.reviews() == 0 {
            ("new".to_string(), Style::new())
        } else {
            let due = summary.due();
            (
                fmt_due(due, now),
                if due < now {
                    Style::new().fg(Color::Red)
                } else {
                    Style::new()
                },
            )
        };

        let interval = if summary.has_schedule() && summary.reviews() > 0 {
            fmt_duration(summary.interval_days())
        } else {
            "-".to_string()
        };
        let stability = match summary.memory() {
            Some(memory) => fmt_duration(memory.stability),
            None => "-".to_string(),
        };
        let difficulty = match summary.memory() {
            Some(memory) => format!("{:.1}", memory.difficulty),
            None => "-".to_string(),
        };
        let (last, last_style) = match summary.last_rating() {
            Some(rating) => (rating.label(), rating_style(rating)),
            None => ("-", Style::new()),
        };
        let (retrievability, retr_style) = match summary.retrievability(now) {
            Some(value) => (fmt_retrievability(value), retrievability_style(value)),
            None => ("-".to_string(), Style::new()),
        };

        let cells = [
            Cell::from(summary.name().to_string()),
            Cell::from(due).style(due_style),
            Cell::from(interval),
            Cell::from(summary.reviews().to_string()),
            Cell::from(stability),
            Cell::from(difficulty),
            Cell::from(last).style(last_style),
            Cell::from(retrievability).style(retr_style),
        ];

        let row = Row::new(cells);
        if index == focus {
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

    if let Some(dialog) = &home.dialog {
        dialog.render(frame.area(), frame.buffer_mut());
    }
}

/// Case-insensitive natural ordering: `item2` sorts before `item10`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().flat_map(char::to_lowercase).peekable();
    let mut b = b.chars().flat_map(char::to_lowercase).peekable();

    loop {
        match (a.peek(), b.peek()) {
            (Some(&first), Some(&second)) => {
                if first.is_ascii_digit() && second.is_ascii_digit() {
                    let (a_number, b_number) = (take_number(&mut a), take_number(&mut b));
                    match a_number.cmp(&b_number) {
                        Ordering::Equal => continue,
                        ordering => return ordering,
                    }
                } else {
                    let first = a.next().expect("peeked");
                    let second = b.next().expect("peeked");
                    match first.cmp(&second) {
                        Ordering::Equal => continue,
                        ordering => return ordering,
                    }
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

fn take_number<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> u64 {
    let mut number = 0u64;
    while let Some(&character) = chars.peek() {
        if character.is_ascii_digit() {
            number = number
                .saturating_mul(10)
                .saturating_add((character as u8 - b'0') as u64);
            chars.next();
        } else {
            break;
        }
    }
    number
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_sorts_numbers() {
        let mut names = ["item10", "item2", "Item1", "item20", "apple", "Apple2"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["apple", "Apple2", "Item1", "item2", "item10", "item20"]
        );
    }

    #[test]
    fn natural_order_is_case_insensitive() {
        assert_eq!(natural_cmp("Apple", "apple"), Ordering::Equal);
        assert_eq!(natural_cmp("B", "a"), Ordering::Greater);
    }
}
