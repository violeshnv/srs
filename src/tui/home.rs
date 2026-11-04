use crate::data::card::{By, Item, Manager, Rating};
use std::{fmt::format, io};

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, Widget},
};

#[derive(Default)]
pub struct Home {
    manager: Manager,
    focus: usize,
    dialog: Option<Dialog>,
    exit: bool,
}

enum Dialog {
    /// name of the to be deleted item
    Remove(String),
    /// name of the to be added item
    Add(String),
    Error(String),
}

type DialogResult = Option<Result<(), ()>>;

impl Dialog {
    /// Result other than Confirm and Cancel is saved in the dialog state.
    /// None means continue this dialog, Some(Ok(())) means confirm, Some(Err(())) means cancel.
    fn handle_key_event(&mut self, event: KeyEvent) -> DialogResult {
        match self {
            Dialog::Remove(_) | Dialog::Error(_) => match event.code {
                KeyCode::Enter => Some(Ok(())),
                KeyCode::Esc => Some(Err(())),
                _ => None,
            },
            Dialog::Add(name) => match event.code {
                KeyCode::Enter => Some(Ok(())),
                KeyCode::Esc => Some(Err(())),
                KeyCode::Char(char) => {
                    name.push(char);
                    None
                }
                KeyCode::Backspace => {
                    name.pop();
                    None
                }
                _ => None,
            },
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let (title, content) = match self {
            Dialog::Remove(name) => (
                "Confirm Delete".to_string(),
                format!("Are you sure you want to delete {name}?"),
            ),
            Dialog::Add(name) => ("Add Item".to_string(), name.clone()),
            Dialog::Error(err) => ("Error".to_string(), err.clone()),
        };
        let instructions = Line::from(vec![
            " Confirm".into(),
            "<Enter>".blue().bold(),
            " Cancel".into(),
            "<Esc>".blue().bold(),
        ]);
        let block = Block::bordered()
            .title(title)
            .title_bottom(instructions.centered())
            .border_set(border::THICK);
        let centered_area = area.centered(Constraint::Percentage(30), Constraint::Percentage(20));
        let paragraph = Paragraph::new(Text::from(content)).block(block);
        paragraph.render(centered_area, buf);
    }
}

impl Home {
    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    /// updates the application's state based on user input
    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                if let Some(dialog) = &mut self.dialog {
                    let result = dialog.handle_key_event(key_event);
                    self.handle_dialog(result);
                } else {
                    self.handle_key_event(key_event)
                }
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_dialog(&mut self, result: DialogResult) {
        let Some(result) = result else {
            return;
        };
        let dialog = self.dialog.take().unwrap();
        match dialog {
            Dialog::Remove(name) => {
                if result.is_ok() {
                    assert!(!self.manager.remove(By::Name(&name)).is_none());
                    self.focus = self.focus.clamp(0, self.manager.histories().len().strict_sub(1))
                }
            }
            Dialog::Add(name) => {
                if result.is_ok() {
                    if let Err(index) = self.manager.add(name) {
                        self.dialog =
                            Some(Dialog::Error(format!("item already existed at {index}")))
                    }
                }
            }
            Dialog::Error(_) => {}
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') | KeyCode::Esc => self.exit(),
            KeyCode::Up => self.change_focus(-1),
            KeyCode::Down => self.change_focus(1),
            KeyCode::Char('d') => self.remove(),
            KeyCode::Char('a') => self.add(),
            KeyCode::Char(c) => {
                match c {
                    '1' => self.update(Rating::Again),
                    '2' => self.update(Rating::Hard),
                    '3' => self.update(Rating::Good),
                    '4' => self.update(Rating::Easy),
                    _ => {}
                };
            }
            _ => {}
        }
    }

    fn update(&mut self, rating: Rating) {
        let Err(err) = self
            .manager
            .review(By::Index(self.focus), rating, Utc::now())
        else {
            return;
        };
        self.dialog = Some(Dialog::Error(format!("Failed to update: {err:?}")));
    }

    fn add(&mut self) {
        self.dialog = Some(Dialog::Add(String::new()))
    }

    fn remove(&mut self) {
        if let Some(h) = self.manager.get(By::Index(self.focus)) {
            self.dialog = Some(Dialog::Remove(h.name().to_string()));
        };
    }

    fn change_focus(&mut self, n: isize) {
        self.focus = self
            .focus
            .saturating_add_signed(n)
            .clamp(0, self.manager.histories().len().strict_sub(1));
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

impl Widget for &Home {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(" App Tutorial ".bold());
        let instructions = Line::from(vec![
            " Add".into(),
            "<A>".blue().bold(),
            " Delete".into(),
            "<D>".blue().bold(),
            " Focus".into(),
            "<Up/Down>".blue().bold(),
            " Quit ".into(),
            "<Q/Esc> ".blue().bold(),
        ]);

        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(border::THICK);

        let header = Row::new(["Name", "Due", "Rating", "Times"]);
        let widths = [
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(10),
        ];

        let histories = self.manager.histories();
        let rows = histories.iter().enumerate().map(|(index, history)| {
            let item = history.items().last().unwrap();
            let cells = [
                history.name().to_string(),
                {
                    let days = item.card.scheduled_days();
                    if days.abs() >= 1.0 {
                        format!("{days:.0} days")
                    } else {
                        format!("{:.1} hours", days * 24.0)
                    }
                },
                format!("{:?}", item.rating),
                history.items().len().to_string(),
            ]
            .map(|cell| {
                let mut cell = Cell::from(cell);
                if index == self.focus {
                    cell = cell.bold().reversed();
                }
                cell
            });
            Row::new(cells)
        });

        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .block(block)
            .render(area, buf);

        if let Some(dialog) = &self.dialog {
            dialog.render(area, buf);
        }
    }
}
