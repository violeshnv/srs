use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};
use fsrs::{FSRS, FSRSError, FSRSItem, FSRSReview, MemoryState};
use serde::{Deserialize, Serialize};

pub const SECONDS_PER_DAY: f32 = 86_400.0;

/// A self-rating for a review, matching FSRS's 1-4 scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    pub const fn value(self) -> u32 {
        match self {
            Self::Again => 1,
            Self::Hard => 2,
            Self::Good => 3,
            Self::Easy => 4,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Again => "Again",
            Self::Hard => "Hard",
            Self::Good => "Good",
            Self::Easy => "Easy",
        }
    }

    /// Parse the shortcut shown in the UI (`1`-`4`).
    pub const fn from_char(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Again),
            '2' => Some(Self::Hard),
            '3' => Some(Self::Good),
            '4' => Some(Self::Easy),
            _ => None,
        }
    }
}

impl fmt::Display for Rating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One review of an item: what the user answered and when.
///
/// This is the only thing persisted. Everything else (memory state, interval,
/// due date) is derived from it, so editing a review automatically replays the
/// schedule for all later reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub when: DateTime<Utc>,
    pub rating: Rating,
}

impl Review {
    pub fn new(when: DateTime<Utc>, rating: Rating) -> Self {
        Self { when, rating }
    }
}

/// A review together with the state it produced.
#[derive(Debug, Clone, Copy)]
pub struct ScheduledReview {
    pub review: Review,
    /// Days since the previous review (fractional, `0` for the first review).
    pub elapsed_days: f32,
    /// Memory state directly after this review.
    pub memory: MemoryState,
    /// Interval scheduled after this review, in days.
    pub interval_days: f32,
    pub due: DateTime<Utc>,
}

/// The full, replayed schedule of an item. The review list is the single
/// source of truth; the current memory state, interval, and due date are
/// derived from its last entry.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub reviews: Vec<ScheduledReview>,
}

impl Schedule {
    fn empty() -> Self {
        Self {
            reviews: Vec::new(),
        }
    }

    pub fn reviews(&self) -> &[ScheduledReview] {
        &self.reviews
    }

    /// Memory state after the most recent review.
    pub fn memory(&self) -> Option<MemoryState> {
        self.reviews.last().map(|review| review.memory)
    }

    /// Interval scheduled after the most recent review, in days.
    pub fn interval_days(&self) -> Option<f32> {
        self.reviews.last().map(|review| review.interval_days)
    }

    /// Due date of the most recent review.
    pub fn due(&self) -> Option<DateTime<Utc>> {
        self.reviews.last().map(|review| review.due)
    }

    /// When the most recent review happened.
    pub fn last_review(&self) -> Option<DateTime<Utc>> {
        self.reviews.last().map(|review| review.review.when)
    }
}

/// Current retrievability of a memory state, using the default FSRS-6 decay.
///
/// The app schedules with default FSRS parameters, so the decay constant is
/// fixed here. Pass a custom decay if parameters are ever optimized.
pub fn retrievability(memory: MemoryState, last_review: DateTime<Utc>, now: DateTime<Utc>) -> f32 {
    let elapsed_days = (now - last_review).num_seconds().max(0) as f32 / SECONDS_PER_DAY;
    fsrs::current_retrievability(memory, elapsed_days, -fsrs::FSRS6_DEFAULT_DECAY)
}

/// One item being learned, together with its complete review history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    name: String,
    #[serde(default = "Utc::now")]
    created: DateTime<Utc>,
    #[serde(default)]
    reviews: Vec<Review>,
}

impl History {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            created: Utc::now(),
            reviews: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn created(&self) -> DateTime<Utc> {
        self.created
    }

    pub fn reviews(&self) -> &[Review] {
        &self.reviews
    }

    pub fn last_review(&self) -> Option<&Review> {
        self.reviews.last()
    }

    pub(crate) fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub(crate) fn push_review(&mut self, review: Review) {
        self.reviews.push(review);
    }

    pub(crate) fn pop_review(&mut self) -> Option<Review> {
        self.reviews.pop()
    }

    pub(crate) fn remove_review(&mut self, index: usize) -> Option<Review> {
        (index < self.reviews.len()).then(|| self.reviews.remove(index))
    }

    /// Returns the previous rating if the index exists.
    pub(crate) fn set_rating(&mut self, index: usize, rating: Rating) -> Option<Rating> {
        let review = self.reviews.get_mut(index)?;
        Some(std::mem::replace(&mut review.rating, rating))
    }

    pub(crate) fn set_review_time(
        &mut self,
        index: usize,
        when: DateTime<Utc>,
    ) -> Result<(), TimeError> {
        if index >= self.reviews.len() {
            return Err(TimeError::NotFound);
        }
        let previous = index.checked_sub(1).map(|i| self.reviews[i].when);
        let next = self.reviews.get(index + 1).map(|r| r.when);
        if next.unwrap_or(DateTime::<Utc>::MIN_UTC) < when
            || when < previous.unwrap_or(DateTime::<Utc>::MAX_UTC)
        {
            return Err(TimeError::OutOfOrder);
        }
        self.reviews[index].when = when;
        Ok(())
    }

    /// Keep reviews in chronological order (used when loading external data).
    pub(crate) fn sort_reviews(&mut self) {
        self.reviews.sort_by_key(|review| review.when);
    }

    /// Replay the review log through FSRS.
    ///
    /// `fsrs.historical_memory_states` returns the memory state after every
    /// review, and `fsrs.next_interval` turns each state into the interval that
    /// was scheduled at that time.
    pub fn schedule(&self, fsrs: &FSRS, desired_retention: f32) -> Result<Schedule, FSRSError> {
        if self.reviews.is_empty() {
            return Ok(Schedule::empty());
        }

        let states = fsrs.historical_memory_states(self.fsrs_item(), None)?;
        let mut scheduled = Vec::with_capacity(self.reviews.len());
        for (index, (review, memory)) in self.reviews.iter().zip(states).enumerate() {
            let elapsed_days = if index == 0 {
                0.0
            } else {
                let previous = self.reviews[index - 1].when;
                (review.when - previous).num_seconds().max(0) as f32 / SECONDS_PER_DAY
            };
            let interval_days = fsrs.next_interval(
                Some(memory.stability),
                desired_retention,
                review.rating.value(),
            );
            let due = review.when + days_to_delta(interval_days);
            scheduled.push(ScheduledReview {
                review: *review,
                elapsed_days,
                memory,
                interval_days,
                due,
            });
        }

        Ok(Schedule { reviews: scheduled })
    }

    fn fsrs_item(&self) -> FSRSItem {
        let reviews = self
            .reviews
            .iter()
            .enumerate()
            .map(|(index, review)| FSRSReview {
                rating: review.rating.value(),
                // The first review is the item's birth; FSRS requires delta_t = 0.
                delta_t: if index == 0 {
                    0
                } else {
                    whole_days(self.reviews[index - 1].when, review.when)
                },
            })
            .collect();
        FSRSItem { reviews }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    NotFound,
    /// The new time would put the review before its predecessor or after its successor.
    OutOfOrder,
}

/// Number of whole days between two timestamps, clamped to `u32`.
fn whole_days(earlier: DateTime<Utc>, later: DateTime<Utc>) -> u32 {
    let seconds = (later - earlier).num_seconds().max(0);
    (seconds / 86_400).min(u32::MAX as i64) as u32
}

/// Convert a fractional number of days to a timestamp delta.
pub fn days_to_delta(days: f32) -> TimeDelta {
    let seconds = (days as f64 * SECONDS_PER_DAY as f64).round();
    let seconds = seconds.clamp(0.0, i64::MAX as f64) as i64;
    TimeDelta::seconds(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn history_with(reviews: &[(i64, Rating)]) -> History {
        let mut history = History::new("test");
        let start = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (days, rating) in reviews {
            history.push_review(Review::new(start + Duration::days(*days), *rating));
        }
        history
    }

    #[test]
    fn replay_produces_states_and_intervals() {
        let history = history_with(&[(0, Rating::Good), (5, Rating::Good)]);
        let schedule = history.schedule(&FSRS::default(), 0.9).unwrap();
        assert_eq!(schedule.reviews.len(), 2);
        assert!(schedule.reviews[0].memory.stability > 0.0);
        assert!(schedule.reviews[1].memory.stability > 0.0);
        assert!(schedule.reviews[1].interval_days > 0.0);
        assert!(schedule.reviews[1].due > schedule.reviews[1].review.when);
        assert_eq!(schedule.reviews[1].elapsed_days, 5.0);
    }

    #[test]
    fn empty_schedule_has_no_state() {
        let history = History::new("new");
        let schedule = history.schedule(&FSRS::default(), 0.9).unwrap();
        assert!(schedule.reviews().is_empty());
        assert!(schedule.memory().is_none());
        assert!(schedule.due().is_none());
        assert!(schedule.interval_days().is_none());
    }

    #[test]
    fn retrievability_is_probability() {
        let history = history_with(&[(0, Rating::Good)]);
        let schedule = history.schedule(&FSRS::default(), 0.9).unwrap();
        let memory = schedule.memory().unwrap();
        let review = history.last_review().unwrap().when;
        let r = retrievability(memory, review, review + Duration::days(1));
        assert!((0.0..=1.0).contains(&r));
    }

    #[test]
    fn set_review_time_rejects_reordering() {
        let mut history = history_with(&[(0, Rating::Good), (5, Rating::Good), (10, Rating::Good)]);
        let start = history.reviews()[0].when;
        assert_eq!(
            history.set_review_time(1, start - Duration::days(1)),
            Err(TimeError::OutOfOrder)
        );
        assert_eq!(
            history.set_review_time(1, start + Duration::days(20)),
            Err(TimeError::OutOfOrder)
        );
        assert!(
            history
                .set_review_time(1, start + Duration::days(2))
                .is_ok()
        );
    }
}
