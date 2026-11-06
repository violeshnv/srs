use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::Style,
    style::Stylize,
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Widget, Wrap},
};

/// The result of a key press while a dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Confirm,
    Cancel,
}

/// One-line text editor with a movable cursor, used by all input dialogs.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// The text before the cursor, the character under the cursor, and the rest.
    fn parts(&self) -> (&str, Option<&str>, &str) {
        let before = &self.value[..self.cursor];
        let rest = &self.value[self.cursor..];
        match rest.chars().next() {
            Some(character) => (
                before,
                Some(&rest[..character.len_utf8()]),
                &rest[character.len_utf8()..],
            ),
            None => (before, None, ""),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('u') {
                self.value.clear();
                self.cursor = 0;
            }
            return;
        }
        match key.code {
            KeyCode::Char(character) if !character.is_control() => {
                self.value.insert(self.cursor, character);
                self.cursor += character.len_utf8();
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = previous_boundary(&self.value, self.cursor);
                self.value.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.value.len() => {
                let next = next_boundary(&self.value, self.cursor);
                self.value.replace_range(self.cursor..next, "");
            }
            KeyCode::Left => self.cursor = previous_boundary(&self.value, self.cursor),
            KeyCode::Right => self.cursor = next_boundary(&self.value, self.cursor),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.value.len(),
            _ => {}
        }
    }
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .chars()
        .next()
        .map(|character| cursor + character.len_utf8())
        .unwrap_or(cursor)
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub title: String,
    pub message: String,
    pub action: ConfirmAction,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    RemoveHistory(String),
    RemoveReview { name: String, index: usize },
    UndoLast(String),
}

pub enum Dialog {
    Add(TextInput),
    Rename {
        old: String,
        input: TextInput,
    },
    SetTime {
        name: String,
        index: usize,
        input: TextInput,
    },
    Confirm(Confirm),
    Error(String),
    Help {
        path: String,
    },
}

impl Dialog {
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Outcome {
        match self {
            Self::Add(input) => text_input_outcome(input, key),
            Self::Rename { input, .. } => text_input_outcome(input, key),
            Self::SetTime { input, .. } => text_input_outcome(input, key),
            Self::Confirm(_) => match key.code {
                KeyCode::Enter => Outcome::Confirm,
                KeyCode::Esc => Outcome::Cancel,
                _ => Outcome::Pending,
            },
            Self::Error(_) => match key.code {
                KeyCode::Enter | KeyCode::Esc => Outcome::Cancel,
                _ => Outcome::Pending,
            },
            Self::Help { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    Outcome::Cancel
                }
                _ => Outcome::Pending,
            },
        }
    }

    pub fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (title, lines, (width, height)) = match self {
            Self::Add(input) => (
                "Add item".to_string(),
                vec![
                    Line::from("Enter a name for the new item:"),
                    Line::from(""),
                    input_line(input),
                ],
                (60, 22),
            ),
            Self::Rename { old, input } => (
                "Rename item".to_string(),
                vec![
                    Line::from(format!("Rename \"{old}\" to:")),
                    Line::from(""),
                    input_line(input),
                ],
                (60, 22),
            ),
            Self::SetTime { index, input, .. } => (
                format!("Edit review #{}", index + 1),
                vec![
                    Line::from("New local time (empty means unchanged):"),
                    Line::from(""),
                    input_line(input),
                    Line::from(""),
                    Line::from("Format: YYYY-MM-DD HH:MM"),
                ],
                (64, 28),
            ),
            Self::Confirm(confirm) => (
                confirm.title.clone(),
                confirm
                    .message
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
                (60, 24),
            ),
            Self::Error(message) => (
                "Error".to_string(),
                message
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
                (64, 26),
            ),
            Self::Help { path } => ("Help".to_string(), help_lines(path), (76, 86)),
        };

        let dialog_area = area.centered(
            Constraint::Percentage(width),
            Constraint::Percentage(height),
        );
        Clear.render(dialog_area, buffer);

        let instructions = if matches!(self, Self::Error(_) | Self::Help { .. }) {
            Line::from(vec![" Close ".into(), "<Enter/Esc>".green().bold()])
        } else {
            Line::from(vec![
                " Confirm ".into(),
                "<Enter>".green().bold(),
                " Cancel ".into(),
                "<Esc>".yellow().bold(),
                " ".into(),
            ])
        };

        let block = Block::bordered()
            .title(format!(" {title} "))
            .title_bottom(instructions.centered())
            .border_set(border::THICK);
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false })
            .render(dialog_area, buffer);
    }
}

fn text_input_outcome(input: &mut TextInput, key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Enter => Outcome::Confirm,
        KeyCode::Esc => Outcome::Cancel,
        _ => {
            input.handle_key(key);
            Outcome::Pending
        }
    }
}

fn input_line(input: &TextInput) -> Line<'_> {
    let (before, under_cursor, after) = input.parts();
    Line::from(vec![
        Span::raw("> "),
        Span::raw(before),
        Span::styled(under_cursor.unwrap_or(" "), Style::new().reversed()),
        Span::raw(after),
    ])
}

fn help_lines(path: &str) -> Vec<Line<'static>> {
    let text = format!(
        "\
Home
  Up/Down, j/k      move selection
  Enter, Right, l   open the item's review history
  1-4               review now (Again / Hard / Good / Easy)
  a                 add item
  e                 rename item
  d                 delete item
  u                 undo the last review
  s                 switch sort order (name / due)
  ?                 this help
  q, Esc            save and quit

History
  Up/Down, j/k      select a review
  1-4               change the selected review's rating
  x, Delete         delete the selected review
  t                 change the selected review's time
  Esc, Left, h, q   back to home

Editing a review replays every later review, so states, intervals and
due dates are always consistent with the review log.

Data file
  {path}

Scheduling
  fsrs 6.6.2 with default parameters and 90% desired retention.
  Review times are stored as UTC and displayed in local time."
    );
    text.lines()
        .map(|line| Line::from(line.to_string()))
        .collect()
}
