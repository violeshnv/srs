use chrono::{DateTime, Utc};

use crate::data::Rating;
use crate::tui::home::Home;

/// A focus-adjustment step a page asks the `App` to run after a mutation.
///
/// The page owns this logic: it captures whatever it needs and hands the `App`
/// a closure that only receives `&mut Home`, so the `App` never reaches into
/// the page's fields directly.
pub(crate) type HomePostWork = Box<dyn FnOnce(&mut Home)>;

/// An action a page asks the [`App`](super::app::App) to perform.
///
/// Pages own their view state and never touch the [`Manager`](crate::data::Manager) directly.
/// Instead they return an [`Event`] and the `App` applies it, the same way dialogs
/// report [`Outcome`](super::dialog::Outcome).
pub(crate) enum Event {
    /// Open the detail page for an item (Home).
    Open(String),
    /// Review an item now (Home).
    Review { name: String, rating: Rating },
    /// Add a new item with the given name (Home).
    Add { name: String, post: HomePostWork },
    /// Rename an item (Home).
    Rename {
        old: String,
        new: String,
        post: HomePostWork,
    },
    /// Delete an item (Home).
    Delete { name: String, post: HomePostWork },
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
