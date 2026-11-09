use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Widget, Wrap},
};

/// The result of a key press while a dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Cancel,
    Confirm,
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
    RemoveReview { index: usize },
    UndoLast(String),
}

pub enum Dialog {
    Add(TextInput),
    Rename { old: String, input: TextInput },
    SetTime { index: usize, input: TextInput },
    Confirm(Confirm),
    Error(String),
    Help { path: String },
}

impl Dialog {
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Option<Outcome> {
        match self {
            Self::Add(input) => text_input_outcome(input, key),
            Self::Rename { input, .. } => text_input_outcome(input, key),
            Self::SetTime { input, .. } => text_input_outcome(input, key),
            Self::Confirm(_) => match key.code {
                KeyCode::Enter => Some(Outcome::Confirm),
                KeyCode::Esc => Some(Outcome::Cancel),
                _ => None,
            },
            Self::Error(_) => match key.code {
                KeyCode::Enter | KeyCode::Esc => Some(Outcome::Cancel),
                _ => None,
            },
            Self::Help { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q' | 'Q' | '?') => {
                    Some(Outcome::Cancel)
                }
                _ => None,
            },
        }
    }

    pub fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (title, lines, width) = match self {
            Self::Add(input) => (
                "Add item".to_string(),
                vec![
                    Line::from("Enter a name for the new item:"),
                    Line::from(""),
                    input_line(input),
                ],
                60,
            ),
            Self::Rename { old, input } => (
                "Rename item".to_string(),
                vec![
                    Line::from(format!("Rename \"{old}\" to:")),
                    Line::from(""),
                    input_line(input),
                ],
                60,
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
                64,
            ),
            Self::Confirm(confirm) => (
                confirm.title.clone(),
                confirm
                    .message
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
                60,
            ),
            Self::Error(message) => (
                "Error".to_string(),
                message
                    .lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
                64,
            ),
            Self::Help { path } => ("Help".to_string(), help_lines(path), 76),
        };

        // Two rows for the top/bottom borders; `Min` lets long wrapped lines
        // grow the dialog instead of clipping them.
        let min_height = lines.len() as u16 + 2;
        let dialog_area = dialog_area(area, width, min_height);
        Clear.render(dialog_area, buffer);

        let instructions = if matches!(self, Self::Error(_) | Self::Help { .. }) {
            Line::from(vec![" Close ".into(), "<ENTER/ESC>".green().bold()])
        } else {
            Line::from(vec![
                " Confirm ".into(),
                "<ENTER>".green().bold(),
                " Cancel ".into(),
                "<ESC>".yellow().bold(),
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

/// A horizontally-centered box whose height is at least `min_height` and grows
/// with its (possibly wrapped) content.
fn dialog_area(area: Rect, width_percent: u16, min_height: u16) -> Rect {
    let horizontal = Layout::horizontal([Constraint::Percentage(width_percent)])
        .flex(Flex::Center)
        .split(area)[0];
    Layout::vertical([
        Constraint::Fill(1),
        Constraint::Min(min_height),
        Constraint::Fill(1),
    ])
    .split(horizontal)[1]
}

fn text_input_outcome(input: &mut TextInput, key: KeyEvent) -> Option<Outcome> {
    match key.code {
        KeyCode::Enter => Some(Outcome::Confirm),
        KeyCode::Esc => Some(Outcome::Cancel),
        _ => {
            input.handle_key(key);
            None
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
  Up/Down, J/K      move selection
  Enter, Right, L   open the item's detail page
  1-4               review now (Again / Hard / Good / Easy)
  A                 add item
  R                 rename item
  D                 delete item
  U                 undo the last review
  S                 cycle sort (due/name, ascending/descending)
  ?                 this help
  Q, Esc            save and quit

Detail
  Up/Down, J/K      select a review
  1-4               change the selected review's rating
  X, Delete         delete the selected review
  T                 change the selected review's time
  Esc, Left, H, Q   back to home

Editing a review replays every later review, so states, intervals and
due dates are always consistent with the review log. Sort is case-insensitive
and uses natural ordering for names (item2 before item10).

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
