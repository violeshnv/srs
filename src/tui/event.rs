use chrono::DateTime;
use chrono::Utc;

use crate::data::Rating;

/// An action a page asks the [`App`](super::app::App) to perform.
///
/// Pages own their view state and never touch the [`Manager`](crate::data::Manager) directly.
/// Instead they return an [`Event`] and the `App` applies it, the same way dialogs
/// report [`Outcome`](super::dialog::Outcome).
#[derive(Debug)]
pub(crate) enum Event {
    /// Open the detail page for an item (Home).
    Open(String),
    /// Review an item now (Home).
    Review { name: String, rating: Rating },
    /// Add a new item with the given name (Home).
    Add(String),
    /// Rename an item (Home).
    Rename { old: String, new: String },
    /// Delete an item (Home).
    Delete(String),
    /// Remove the most recent review (Home).
    Undo(String),
    /// Save and exit (Home).
    Quit,
    /// Show the help dialog on the active page.
    Help,
    /// Return from the detail page to home.
    Back,
    /// Change the rating of the selected review (Detail).
    SetRating { index: usize, rating: Rating },
    /// Delete the selected review (Detail).
    DeleteReview(usize),
    /// Move the selected review to a new time (Detail).
    EditTime { index: usize, when: DateTime<Utc> },
}
