use std::{io, path::PathBuf};

use chrono::{DateTime, Utc};
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame, widgets::Paragraph};

use crate::data::{Manager, Rating};

use super::{
    Status,
    detail::{self, Detail},
    dialog::Dialog,
    event::Event,
    fmt_when,
    home::{self, Home, Summary},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Active {
    Home,
    Detail,
}

/// Owns the data and the two concrete pages. Each page keeps its own state
/// (selection, sort, dialogs, status); `active` only says which one is shown.
pub struct App {
    manager: Manager,
    home: Home,
    detail: Detail,
    active: Active,
    exit: bool,
}

impl App {
    pub fn new(path: PathBuf) -> Self {
        let (manager, notice) = Manager::load(path);
        let mut home = Home::default();
        match notice {
            Some(notice) if notice.error => home.dialog = Some(Dialog::Error(notice.message)),
            Some(notice) => home.status = Some(Status::info(notice.message)),
            None => {}
        }
        Self {
            manager,
            home,
            detail: Detail::new(String::new(), 0),
            active: Active::Home,
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
        match self.active {
            Active::Home => {
                let file_name = self.file_name();
                let schedules = self.manager.schedules();
                let summaries: Vec<Summary<'_>> = self
                    .manager
                    .histories()
                    .iter()
                    .zip(&schedules)
                    .map(|(history, schedule)| Summary::new(history, schedule.as_ref()))
                    .collect();
                let ordered = self.home.order(&summaries);
                home::render(frame, &self.home, &ordered, &file_name);
            }
            Active::Detail => {
                let name = self.detail.name.clone();
                let history = self.manager.get(&name);
                let schedule = history.and_then(|history| {
                    history
                        .schedule(self.manager.fsrs(), self.manager.desired_retention())
                        .ok()
                });
                match history {
                    Some(history) => detail::render(
                        frame,
                        &self.detail,
                        history,
                        schedule.as_ref(),
                        self.manager.fsrs(),
                        self.manager.desired_retention(),
                    ),
                    None => {
                        frame.render_widget(
                            Paragraph::new(format!(
                                "Item \"{name}\" no longer exists. Press <ESC>."
                            )),
                            frame.area(),
                        );
                    }
                }
            }
        }
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let CrosstermEvent::Key(key) = event::read()? else {
            return Ok(());
        };
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit();
            return Ok(());
        }

        let event = match self.active {
            Active::Home => {
                let schedules = self.manager.schedules();
                let summaries: Vec<Summary<'_>> = self
                    .manager
                    .histories()
                    .iter()
                    .zip(&schedules)
                    .map(|(history, schedule)| Summary::new(history, schedule.as_ref()))
                    .collect();
                let ordered = self.home.order(&summaries);
                self.home.handle_key(key, &ordered)
            }
            Active::Detail => {
                let name = self.detail.name.clone();
                match self.manager.get(&name) {
                    Some(history) => self.detail.handle_key(key, history),
                    None => None,
                }
            }
        };

        if let Some(event) = event {
            self.apply(event);
        }
        Ok(())
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Open(name) => self.open_detail(name),
            Event::Review { name, rating } => self.review(&name, rating),
            Event::Add(name) => self.add(&name),
            Event::Rename { old, new } => self.rename(&old, &new),
            Event::Delete(name) => self.delete(&name),
            Event::Undo(name) => self.undo(&name),
            Event::Quit => self.quit(),
            Event::Help => self.show_help(),
            Event::Back => {
                self.active = Active::Home;
                self.detail.dialog = None;
                self.detail.status = None;
            }
            Event::SetRating { index, rating } => self.set_rating(index, rating),
            Event::DeleteReview(index) => self.delete_review(index),
            Event::EditTime { index, when } => self.edit_time(index, when),
        }
    }

    fn file_name(&self) -> String {
        self.manager
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.manager.path().display().to_string())
    }

    fn open_detail(&mut self, name: String) {
        if self.detail.name != name {
            let focus = self
                .manager
                .get(&name)
                .map(|history| history.reviews().len().saturating_sub(1))
                .unwrap_or(0);
            self.detail = Detail::new(name, focus);
        }
        self.active = Active::Detail;
    }

    fn review(&mut self, name: &str, rating: Rating) {
        match self.manager.review(name, rating, Utc::now()) {
            Ok(()) => {
                self.home.status = Some(Status::info(format!("Reviewed \"{name}\": {rating}")));
                self.save();
            }
            Err(error) => self.home.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn add(&mut self, name: &str) {
        match self.manager.add(name) {
            Ok(()) => {
                self.home.focus_on(name);
                self.home.status = Some(Status::info(format!("Added \"{name}\"")));
                self.save();
            }
            Err(error) => self.home.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn rename(&mut self, old: &str, new: &str) {
        match self.manager.rename(old, new) {
            Ok(()) => {
                self.home.focus_on(new);
                self.home.status = Some(Status::info(format!("Renamed \"{old}\" to \"{new}\"")));
                self.save();
            }
            Err(error) => self.home.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn delete(&mut self, name: &str) {
        let next_focus = self.home_focus_after_removal(name);
        if self.manager.remove(name) {
            match next_focus {
                Some(next) => self.home.focus_on(&next),
                None => self.home.clear_focus(),
            }
            self.home.status = Some(Status::info(format!("Deleted \"{name}\"")));
            self.save();
        }
    }

    fn undo(&mut self, name: &str) {
        match self.manager.undo_last(name) {
            Ok(()) => {
                self.home.status =
                    Some(Status::info(format!("Undid the last review of \"{name}\"")));
                self.save();
            }
            Err(error) => self.home.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn set_rating(&mut self, index: usize, rating: Rating) {
        let name = self.detail.name.clone();
        match self.manager.set_rating(&name, index, rating) {
            Ok(()) => {
                self.detail.status = Some(Status::info(format!(
                    "Review #{} -> {rating}; later reviews replayed",
                    index + 1
                )));
                self.save();
            }
            Err(error) => self.detail.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn delete_review(&mut self, index: usize) {
        let name = self.detail.name.clone();
        match self.manager.delete_review(&name, index) {
            Ok(()) => {
                if self.detail.focus > index {
                    self.detail.focus -= 1;
                }
                let len = self
                    .manager
                    .get(&name)
                    .map(|history| history.reviews().len())
                    .unwrap_or(0);
                self.detail.focus = self.detail.focus.min(len.saturating_sub(1));
                self.detail.status = Some(Status::info(format!("Deleted review #{}", index + 1)));
                self.save();
            }
            Err(error) => self.detail.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    fn edit_time(&mut self, index: usize, when: DateTime<Utc>) {
        let name = self.detail.name.clone();
        match self.manager.set_review_time(&name, index, when) {
            Ok(()) => {
                self.detail.status = Some(Status::info(format!(
                    "Moved review #{} to {}; later reviews replayed",
                    index + 1,
                    fmt_when(when)
                )));
                self.save();
            }
            Err(error) => self.detail.dialog = Some(Dialog::Error(error.to_string())),
        }
    }

    /// The item to focus after `name` is removed: the current selection if it
    /// survives, otherwise its neighbour in the sorted list.
    fn home_focus_after_removal(&self, removed: &str) -> Option<String> {
        let schedules = self.manager.schedules();
        let summaries: Vec<Summary<'_>> = self
            .manager
            .histories()
            .iter()
            .zip(&schedules)
            .map(|(history, schedule)| Summary::new(history, schedule.as_ref()))
            .collect();
        let ordered = self.home.order(&summaries);
        let current = self.home.focused_index(&ordered);
        let current_name = ordered
            .get(current)
            .map(|summary| summary.name().to_string());

        let remaining: Vec<&Summary<'_>> = ordered
            .iter()
            .filter(|summary| summary.name() != removed)
            .collect();
        if remaining.is_empty() {
            return None;
        }

        match current_name {
            Some(name) if name != removed => Some(name),
            _ => remaining
                .get(current.min(remaining.len() - 1))
                .map(|summary| summary.name().to_string()),
        }
    }

    fn show_help(&mut self) {
        let dialog = Dialog::Help {
            path: self.manager.path().display().to_string(),
        };
        match self.active {
            Active::Home => self.home.dialog = Some(dialog),
            Active::Detail => self.detail.dialog = Some(dialog),
        }
    }

    fn save(&mut self) {
        if let Err(error) = self.manager.save() {
            let dialog = Dialog::Error(format!(
                "Could not save data to {}:\n{error}",
                self.manager.path().display()
            ));
            self.show_dialog(dialog);
        }
    }

    fn quit(&mut self) {
        match self.manager.save() {
            Ok(()) => self.exit = true,
            Err(error) => {
                let dialog = Dialog::Error(format!(
                    "Could not save data to {}:\n{error}\n\nPress <ESC> to close this dialog, then try again.",
                    self.manager.path().display()
                ));
                self.show_dialog(dialog);
            }
        }
    }

    fn show_dialog(&mut self, dialog: Dialog) {
        match self.active {
            Active::Home => self.home.dialog = Some(dialog),
            Active::Detail => self.detail.dialog = Some(dialog),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    use crate::tui::dialog::TextInput;
    use crate::tui::home::Sort;

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

        app.active = Active::Detail;
        app.detail = Detail::new("kanji".to_string(), 0);
        render(&app, 120, 30);
        render(&app, 40, 12);

        app.active = Active::Home;
        app.home.dialog = Some(Dialog::Add(TextInput::new("new item")));
        render(&app, 120, 30);
        app.home.dialog = Some(Dialog::Help {
            path: "srs.ron".to_string(),
        });
        render(&app, 120, 30);
        app.home.dialog = Some(Dialog::Error("something went wrong".to_string()));
        render(&app, 120, 30);
    }

    #[test]
    fn focus_is_stable_across_sort_changes() {
        let mut app = app();
        app.manager.add("item10").unwrap();
        app.manager.add("item2").unwrap();
        app.home.focus_on("item2");

        let focused_name = |app: &App| {
            let schedules = app.schedules();
            let summaries: Vec<Summary<'_>> = app
                .manager
                .histories()
                .iter()
                .zip(&schedules)
                .map(|(history, schedule)| Summary::new(history, schedule.as_ref()))
                .collect();
            let ordered = app.home.order(&summaries);
            ordered[app.home.focused_index(&ordered)].name().to_string()
        };

        app.home.set_sort(Sort::NameAsc);
        assert_eq!(focused_name(&app), "item2");
        app.home.set_sort(Sort::NameDesc);
        assert_eq!(focused_name(&app), "item2");
        app.home.set_sort(Sort::DueAsc);
        assert_eq!(focused_name(&app), "item2");
        app.home.set_sort(Sort::DueDesc);
        assert_eq!(focused_name(&app), "item2");
    }

    #[test]
    fn focus_survives_a_review_that_changes_order() {
        let mut app = app();
        app.home.focus_on("vocab");
        // Reviewing changes "vocab"'s due date; the name-based cursor must not move.
        app.review("vocab", Rating::Good);

        let schedules = app.schedules();
        let summaries: Vec<Summary<'_>> = app
            .manager
            .histories()
            .iter()
            .zip(&schedules)
            .map(|(history, schedule)| Summary::new(history, schedule.as_ref()))
            .collect();
        let ordered = app.home.order(&summaries);
        assert_eq!(ordered[app.home.focused_index(&ordered)].name(), "vocab");
    }
}
