use std::{io, path::PathBuf};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

use crate::data::{History, Manager, Rating, Summary};

use super::{
    Status, dialog::Confirm, dialog::ConfirmAction, dialog::Dialog, dialog::Outcome,
    dialog::TextInput, fmt_when, history, home,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortBy {
    Name,
    Due,
}

impl SortBy {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Due,
            Self::Due => Self::Name,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Due => "due",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Page {
    Home,
    History { name: String, focus: usize },
}

pub struct App {
    pub(crate) manager: Manager,
    pub(crate) page: Page,
    /// Selected row on the home page (index into the sorted view).
    pub(crate) focus: usize,
    pub(crate) sort: SortBy,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) status: Option<Status>,
    exit: bool,
}

impl App {
    pub fn new(path: PathBuf) -> Self {
        let (manager, notice) = Manager::load(path);
        let (status, dialog) = match notice {
            Some(notice) if notice.error => (None, Some(Dialog::Error(notice.message))),
            Some(notice) => (Some(Status::info(notice.message)), None),
            None => (None, None),
        };
        Self {
            manager,
            page: Page::Home,
            focus: 0,
            sort: SortBy::Due,
            dialog,
            status,
            exit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        match &self.page {
            Page::Home => home::render(self, frame),
            Page::History { name, focus } => history::render(self, name, *focus, frame),
        }
        if let Some(dialog) = &self.dialog {
            dialog.render(frame.area(), frame.buffer_mut());
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let Event::Key(key) = event::read()? else {
            return Ok(());
        };
        // Crossterm also emits release/repeat events on some platforms.
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        self.status = None;

        if self.dialog.is_some() {
            let outcome = self
                .dialog
                .as_mut()
                .expect("dialog was checked above")
                .handle_key_event(key);
            if outcome != Outcome::Pending {
                self.apply_dialog(outcome);
            }
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit();
            return Ok(());
        }

        match self.page.clone() {
            Page::Home => self.handle_home_key(key),
            Page::History { .. } => self.handle_history_key(key),
        }
        Ok(())
    }

    fn handle_home_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Up | KeyCode::Char('k') => self.move_focus(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_focus(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.open_history(),
            KeyCode::Char('a') => self.dialog = Some(Dialog::Add(TextInput::default())),
            KeyCode::Char('e') => self.rename_dialog(),
            KeyCode::Char('d') => self.delete_history_dialog(),
            KeyCode::Char('u') => self.undo_dialog(),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('?') => self.show_help(),
            KeyCode::Char(character @ '1'..='4') => {
                if let Some(rating) = Rating::from_char(character) {
                    self.review(rating);
                }
            }
            _ => {}
        }
    }

    fn handle_history_key(&mut self, key: KeyEvent) {
        let Page::History { name, focus } = self.page.clone() else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('q') => {
                self.page = Page::Home;
            }
            KeyCode::Up | KeyCode::Char('k') => self.set_history_focus(focus.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.set_history_focus(focus.saturating_add(1)),
            KeyCode::Char(character @ '1'..='4') => {
                if let Some(rating) = Rating::from_char(character) {
                    self.change_rating(&name, focus, rating);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => self.delete_review_dialog(&name, focus),
            KeyCode::Char('t') => self.edit_time_dialog(&name, focus),
            KeyCode::Char('?') => self.show_help(),
            _ => {}
        }
    }

    fn apply_dialog(&mut self, outcome: Outcome) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        if outcome == Outcome::Cancel {
            return;
        }

        match dialog {
            Dialog::Add(input) => {
                let name = input.value().trim().to_string();
                match self.manager.add(&name) {
                    Ok(()) => {
                        self.focus_on(&name);
                        self.status = Some(Status::info(format!("Added \"{name}\"")));
                        self.save();
                    }
                    Err(error) => self.show_error(error),
                }
            }
            Dialog::Rename { old, input } => {
                let new = input.value().trim().to_string();
                match self.manager.rename(&old, &new) {
                    Ok(()) => {
                        self.focus_on(&new);
                        self.status = Some(Status::info(format!("Renamed \"{old}\" to \"{new}\"")));
                        self.save();
                    }
                    Err(error) => self.show_error(error),
                }
            }
            Dialog::SetTime { name, index, input } => {
                if input.value().trim().is_empty() {
                    return;
                }
                let Some(when) = parse_local_time(input.value()) else {
                    self.show_message(
                        "Could not parse the time. Use YYYY-MM-DD HH:MM (local time).",
                    );
                    return;
                };
                match self.manager.set_review_time(&name, index, when) {
                    Ok(()) => {
                        self.status = Some(Status::info(format!(
                            "Moved review #{} to {}; later reviews replayed",
                            index + 1,
                            fmt_when(when)
                        )));
                        self.save();
                    }
                    Err(error) => self.show_error(error),
                }
            }
            Dialog::Confirm(confirm) => match confirm.action {
                ConfirmAction::RemoveHistory(name) => {
                    if self.manager.remove(&name) {
                        self.clamp_focus();
                        self.status = Some(Status::info(format!("Deleted \"{name}\"")));
                        self.save();
                    }
                }
                ConfirmAction::RemoveReview { name, index } => {
                    match self.manager.delete_review(&name, index) {
                        Ok(()) => {
                            self.adjust_history_focus(&name, index);
                            self.status =
                                Some(Status::info(format!("Deleted review #{}", index + 1)));
                            self.save();
                        }
                        Err(error) => self.show_error(error),
                    }
                }
                ConfirmAction::UndoLast(name) => match self.manager.undo_last(&name) {
                    Ok(()) => {
                        self.status =
                            Some(Status::info(format!("Undid the last review of \"{name}\"")));
                        self.save();
                    }
                    Err(error) => self.show_error(error),
                },
            },
            Dialog::Error(_) | Dialog::Help { .. } => {}
        }
    }

    /// Rows of the home page, sorted according to `self.sort`.
    pub(crate) fn ordered_rows(&self) -> Vec<(&History, Option<Summary>)> {
        let mut rows: Vec<_> = self
            .manager
            .histories()
            .iter()
            .map(|history| (history, self.manager.summary(history.name()).ok()))
            .collect();
        match self.sort {
            SortBy::Name => rows.sort_by(|a, b| {
                a.0.name()
                    .to_lowercase()
                    .cmp(&b.0.name().to_lowercase())
                    .then_with(|| a.0.name().cmp(b.0.name()))
            }),
            SortBy::Due => rows.sort_by(|a, b| {
                let a_due =
                    a.1.map(|summary| summary.due)
                        .unwrap_or_else(|| a.0.created());
                let b_due =
                    b.1.map(|summary| summary.due)
                        .unwrap_or_else(|| b.0.created());
                a_due.cmp(&b_due).then_with(|| a.0.name().cmp(b.0.name()))
            }),
        }
        rows
    }

    fn focused_name(&self) -> Option<String> {
        self.ordered_rows()
            .get(self.focus)
            .map(|(history, _)| history.name().to_string())
    }

    fn focus_on(&mut self, name: &str) {
        if let Some(index) = self
            .ordered_rows()
            .iter()
            .position(|(history, _)| history.name() == name)
        {
            self.focus = index;
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let len = self.manager.histories().len();
        if len == 0 {
            self.focus = 0;
        } else {
            self.focus = self.focus.saturating_add_signed(delta).min(len - 1);
        }
    }

    fn clamp_focus(&mut self) {
        let len = self.manager.histories().len();
        self.focus = if len == 0 { 0 } else { self.focus.min(len - 1) };
    }

    fn set_history_focus(&mut self, focus: usize) {
        if let Page::History {
            name,
            focus: current,
        } = &mut self.page
        {
            let len = self
                .manager
                .get(name)
                .map(|history| history.reviews().len())
                .unwrap_or(0);
            *current = focus.min(len.saturating_sub(1));
        }
    }

    fn adjust_history_focus(&mut self, name: &str, deleted: usize) {
        if let Page::History {
            name: current_name,
            focus,
        } = &mut self.page
        {
            if current_name != name {
                return;
            }
            if *focus > deleted {
                *focus -= 1;
            }
            let len = self
                .manager
                .get(name)
                .map(|history| history.reviews().len())
                .unwrap_or(0);
            *focus = (*focus).min(len.saturating_sub(1));
        }
    }

    fn cycle_sort(&mut self) {
        let focused = self.focused_name();
        self.sort = self.sort.next();
        match focused {
            Some(name) => self.focus_on(&name),
            None => self.focus = 0,
        }
    }

    fn open_history(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let focus = self
            .manager
            .get(&name)
            .map(|history| history.reviews().len().saturating_sub(1))
            .unwrap_or(0);
        self.page = Page::History { name, focus };
    }

    fn review(&mut self, rating: Rating) {
        let Some(name) = self.focused_name() else {
            return;
        };
        match self.manager.review(&name, rating, Utc::now()) {
            Ok(()) => {
                self.status = Some(Status::info(format!("Reviewed \"{name}\": {rating}")));
                self.save();
            }
            Err(error) => self.show_error(error),
        }
    }

    fn change_rating(&mut self, name: &str, index: usize, rating: Rating) {
        if self
            .manager
            .get(name)
            .is_some_and(|history| history.reviews().is_empty())
        {
            self.status = Some(Status::info("This item has no reviews yet"));
            return;
        }
        match self.manager.set_rating(name, index, rating) {
            Ok(()) => {
                self.status = Some(Status::info(format!(
                    "Review #{} -> {rating}; later reviews replayed",
                    index + 1
                )));
                self.save();
            }
            Err(error) => self.show_error(error),
        }
    }

    fn rename_dialog(&mut self) {
        if let Some(name) = self.focused_name() {
            self.dialog = Some(Dialog::Rename {
                old: name,
                input: TextInput::default(),
            });
        }
    }

    fn delete_history_dialog(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let reviews = self
            .manager
            .get(&name)
            .map(|history| history.reviews().len())
            .unwrap_or(0);
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Delete item".to_string(),
            message: format!(
                "Delete \"{name}\" and its {reviews} review(s)?\nThis cannot be undone."
            ),
            action: ConfirmAction::RemoveHistory(name),
        }));
    }

    fn undo_dialog(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(review) = self
            .manager
            .get(&name)
            .and_then(History::last_review)
            .copied()
        else {
            self.status = Some(Status::info("Nothing to undo for this item"));
            return;
        };
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Undo last review".to_string(),
            message: format!(
                "Remove the last review of \"{name}\" ({} at {})?",
                review.rating,
                fmt_when(review.when)
            ),
            action: ConfirmAction::UndoLast(name),
        }));
    }

    fn delete_review_dialog(&mut self, name: &str, index: usize) {
        let Some(review) = self
            .manager
            .get(name)
            .and_then(|history| history.reviews().get(index))
            .copied()
        else {
            self.status = Some(Status::info("This item has no reviews yet"));
            return;
        };
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Delete review".to_string(),
            message: format!(
                "Delete review #{} ({}, {})?\nLater reviews will be replayed.",
                index + 1,
                review.rating,
                fmt_when(review.when)
            ),
            action: ConfirmAction::RemoveReview {
                name: name.to_string(),
                index,
            },
        }));
    }

    fn edit_time_dialog(&mut self, name: &str, index: usize) {
        let Some(review) = self
            .manager
            .get(name)
            .and_then(|history| history.reviews().get(index))
            .copied()
        else {
            self.status = Some(Status::info("This item has no reviews yet"));
            return;
        };
        let local = review
            .when
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        self.dialog = Some(Dialog::SetTime {
            name: name.to_string(),
            index,
            input: TextInput::new(local),
        });
    }

    fn show_help(&mut self) {
        self.dialog = Some(Dialog::Help {
            path: self.manager.path().display().to_string(),
        });
    }

    fn show_error(&mut self, error: impl std::fmt::Display) {
        self.dialog = Some(Dialog::Error(error.to_string()));
    }

    fn show_message(&mut self, message: &str) {
        self.dialog = Some(Dialog::Error(message.to_string()));
    }

    fn save(&mut self) {
        if let Err(error) = self.manager.save() {
            self.dialog = Some(Dialog::Error(format!(
                "Could not save data to {}:\n{error}",
                self.manager.path().display()
            )));
        }
    }

    fn quit(&mut self) {
        match self.manager.save() {
            Ok(()) => self.exit = true,
            Err(error) => {
                self.dialog = Some(Dialog::Error(format!(
                    "Could not save data to {}:\n{error}\n\nPress <Esc> to close this dialog, then try again.",
                    self.manager.path().display()
                )))
            }
        }
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
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        let path = std::env::temp_dir().join(format!(
            "srs-tui-test-{}-{}.ron",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = App::new(path);
        app.manager.add("kanji").unwrap();
        app.manager.add("vocab").unwrap();
        app.manager
            .review("kanji", Rating::Good, Utc::now())
            .unwrap();
        app
    }

    fn render(app: &App, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }

    #[test]
    fn renders_every_page_and_dialog() {
        let mut app = app();
        render(&app, 120, 30);
        render(&app, 40, 12);

        app.page = Page::History {
            name: "kanji".to_string(),
            focus: 0,
        };
        render(&app, 120, 30);
        render(&app, 40, 12);

        app.dialog = Some(Dialog::Add(TextInput::new("new item")));
        render(&app, 120, 30);
        app.dialog = Some(Dialog::Help {
            path: "srs.ron".to_string(),
        });
        render(&app, 120, 30);
        app.dialog = Some(Dialog::Error("something went wrong".to_string()));
        render(&app, 120, 30);
    }

    #[test]
    fn parses_local_times() {
        assert!(parse_local_time("2026-11-05 12:30").is_some());
        assert!(parse_local_time("2026-11-05").is_some());
        assert!(parse_local_time("  ").is_none());
        assert!(parse_local_time("not a date").is_none());
    }
}
