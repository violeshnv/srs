pub mod app;
pub mod detail;
pub mod dialog;
pub mod event;
pub mod home;

use chrono::{DateTime, Local, Utc};
use ratatui::style::{Color, Style};

use crate::data::Rating;

/// A short, non-blocking message shown in the corner of the current page.
#[derive(Debug, Clone)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

impl Status {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    pub fn style(&self) -> Style {
        Style::new().fg(if self.error { Color::Red } else { Color::Green })
    }
}

pub fn rating_style(rating: Rating) -> Style {
    let color = match rating {
        Rating::Again => Color::Red,
        Rating::Hard => Color::Yellow,
        Rating::Good => Color::Green,
        Rating::Easy => Color::Cyan,
    };
    Style::new().fg(color)
}

/// Format a duration in days compactly, e.g. `3.2d`, `5h`, `<1h`.
pub fn fmt_duration(days: f32) -> String {
    let days = days.abs();
    if days < 1.0 / 24.0 {
        "<1h".to_string()
    } else if days < 1.0 {
        format!("{:.0}h", days * 24.0)
    } else if days < 30.0 {
        format!("{days:.1}d")
    } else if days < 365.0 {
        format!("{days:.0}d")
    } else {
        format!("{:.1}y", days / 365.25)
    }
}

/// Format a due date relative to `now`.
pub fn fmt_due(due: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let delta_days = (due - now).num_seconds() as f32 / 86_400.0;
    if delta_days.abs() < 1.0 / 1_440.0 {
        "now".to_string()
    } else if delta_days < 0.0 {
        format!("{} late", fmt_duration(delta_days))
    } else {
        format!("in {}", fmt_duration(delta_days))
    }
}

/// Timestamps are stored as UTC but shown in the user's local timezone.
pub fn fmt_when(when: DateTime<Utc>) -> String {
    when.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

pub fn fmt_retrievability(retrievability: f32) -> String {
    format!("{:.0}%", retrievability * 100.0)
}

pub fn retrievability_style(retrievability: f32) -> Style {
    let color = if retrievability >= 0.9 {
        Color::Green
    } else if retrievability >= 0.8 {
        Color::Yellow
    } else {
        Color::Red
    };
    Style::new().fg(color)
}
