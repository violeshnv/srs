use std::{
    collections::HashSet,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fsrs::{FSRS, FSRSError};
use serde::{Deserialize, Serialize};

use super::history::{History, Rating, Review, Summary, TimeError};

pub const DEFAULT_DESIRED_RETENTION: f32 = 0.9;
pub const DEFAULT_DATA_PATH: &str = "srs.ron";

/// On-disk format. Keep this stable and human-readable: the file is meant to be
/// edited and inspected by hand as well as by the app.
#[derive(Debug, Serialize, Deserialize)]
struct Store {
    #[serde(default = "Store::version")]
    version: u32,
    #[serde(default = "default_desired_retention")]
    desired_retention: f32,
    #[serde(default)]
    histories: Vec<History>,
}

impl Store {
    const CURRENT_VERSION: u32 = 1;

    const fn version() -> u32 {
        Self::CURRENT_VERSION
    }
}

fn default_desired_retention() -> f32 {
    DEFAULT_DESIRED_RETENTION
}

/// A non-fatal message produced while loading data.
#[derive(Debug, Clone)]
pub struct Notice {
    pub message: String,
    pub error: bool,
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    EmptyName,
    Duplicate(String),
    InvalidTime,
    Fsrs(FSRSError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("Item not found"),
            Self::EmptyName => f.write_str("Name cannot be empty"),
            Self::Duplicate(name) => write!(f, "An item named \"{name}\" already exists"),
            Self::InvalidTime => {
                f.write_str("That time is before the previous review or after the next one")
            }
            Self::Fsrs(error) => write!(f, "FSRS error: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<FSRSError> for Error {
    fn from(error: FSRSError) -> Self {
        Self::Fsrs(error)
    }
}

impl From<TimeError> for Error {
    fn from(error: TimeError) -> Self {
        match error {
            TimeError::NotFound => Self::NotFound,
            TimeError::OutOfOrder => Self::InvalidTime,
        }
    }
}

/// Owns the FSRS model and every item. All mutations go through here so that
/// name uniqueness and review ordering stay valid.
pub struct Manager {
    fsrs: FSRS,
    desired_retention: f32,
    histories: Vec<History>,
    path: PathBuf,
}

impl Manager {
    pub fn new(path: PathBuf) -> Self {
        Self {
            fsrs: FSRS::default(),
            desired_retention: DEFAULT_DESIRED_RETENTION,
            histories: Vec::new(),
            path,
        }
    }

    /// Load the RON file at `path`. A missing file is not an error; it simply
    /// starts with an empty collection.
    pub fn load(path: PathBuf) -> (Self, Option<Notice>) {
        let mut manager = Self::new(path.clone());
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return (
                    manager,
                    Some(Notice {
                        message: format!("No data file at {} — starting fresh", path.display()),
                        error: false,
                    }),
                );
            }
            Err(error) => {
                return (
                    manager,
                    Some(Notice {
                        message: format!("Could not read {}: {error}", path.display()),
                        error: true,
                    }),
                );
            }
        };

        if text.trim().is_empty() {
            return (
                manager,
                Some(Notice {
                    message: format!("{} is empty — starting fresh", path.display()),
                    error: false,
                }),
            );
        }

        match ron::from_str::<Store>(&text) {
            Ok(store) => {
                if store.desired_retention.is_finite() {
                    manager.desired_retention = store.desired_retention.clamp(0.70, 0.99);
                }
                manager.histories = store.histories;
                let dropped = manager.sanitize();
                let mut messages = Vec::new();
                if store.version > Store::CURRENT_VERSION {
                    messages.push(format!(
                        "File version {} is newer than supported version {}",
                        store.version,
                        Store::CURRENT_VERSION
                    ));
                }
                if dropped > 0 {
                    messages.push(format!(
                        "Ignored {dropped} duplicate item name(s) while loading"
                    ));
                }
                let notice = (!messages.is_empty()).then(|| Notice {
                    message: messages.join("; "),
                    error: false,
                });
                (manager, notice)
            }
            Err(error) => (
                manager,
                Some(Notice {
                    message: format!("Could not parse {}: {error}", path.display()),
                    error: true,
                }),
            ),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn fsrs(&self) -> &FSRS {
        &self.fsrs
    }

    pub fn desired_retention(&self) -> f32 {
        self.desired_retention
    }

    pub fn histories(&self) -> &[History] {
        &self.histories
    }

    pub fn get(&self, name: &str) -> Option<&History> {
        self.histories.iter().find(|history| history.name() == name)
    }

    /// Add a new, never-reviewed item.
    pub fn add(&mut self, name: &str) -> Result<(), Error> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        if self.get(name).is_some() {
            return Err(Error::Duplicate(name.to_string()));
        }
        self.histories.push(History::new(name));
        self.sort_by_name();
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self
            .histories
            .iter()
            .position(|history| history.name() == name)
        else {
            return false;
        };
        self.histories.remove(index);
        true
    }

    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), Error> {
        let new = new.trim();
        if new.is_empty() {
            return Err(Error::EmptyName);
        }
        if new != old && self.get(new).is_some() {
            return Err(Error::Duplicate(new.to_string()));
        }
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == old)
            .ok_or(Error::NotFound)?;
        history.set_name(new);
        self.sort_by_name();
        Ok(())
    }

    /// Record a review of `name` at `when`.
    pub fn review(&mut self, name: &str, rating: Rating, when: DateTime<Utc>) -> Result<(), Error> {
        let fsrs = &self.fsrs;
        let desired_retention = self.desired_retention;
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == name)
            .ok_or(Error::NotFound)?;

        if history.last_review().is_some_and(|last| when < last.when) {
            return Err(Error::InvalidTime);
        }

        history.push_review(Review::new(when, rating));
        if let Err(error) = history.schedule(fsrs, desired_retention) {
            history.pop_review();
            return Err(error.into());
        }
        Ok(())
    }

    /// Change the rating of an existing review. Later reviews are replayed
    /// automatically.
    pub fn set_rating(&mut self, name: &str, index: usize, rating: Rating) -> Result<(), Error> {
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == name)
            .ok_or(Error::NotFound)?;
        history.set_rating(index, rating).ok_or(Error::NotFound)?;
        Ok(())
    }

    /// Move an existing review in time. The new time has to stay between the
    /// neighbouring reviews.
    pub fn set_review_time(
        &mut self,
        name: &str,
        index: usize,
        when: DateTime<Utc>,
    ) -> Result<(), Error> {
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == name)
            .ok_or(Error::NotFound)?;
        history.set_review_time(index, when)?;
        Ok(())
    }

    pub fn delete_review(&mut self, name: &str, index: usize) -> Result<(), Error> {
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == name)
            .ok_or(Error::NotFound)?;
        history.remove_review(index).ok_or(Error::NotFound)?;
        Ok(())
    }

    /// Remove the most recent review.
    pub fn undo_last(&mut self, name: &str) -> Result<(), Error> {
        let history = self
            .histories
            .iter_mut()
            .find(|history| history.name() == name)
            .ok_or(Error::NotFound)?;
        history.pop_review().ok_or(Error::NotFound)?;
        Ok(())
    }

    pub fn summary(&self, name: &str) -> Result<Summary, Error> {
        let history = self.get(name).ok_or(Error::NotFound)?;
        history
            .summary(&self.fsrs, self.desired_retention)
            .map_err(Error::Fsrs)
    }

    /// Serialize everything to `self.path`.
    pub fn save(&self) -> Result<(), String> {
        let store = Store {
            version: Store::CURRENT_VERSION,
            desired_retention: self.desired_retention,
            histories: self.histories.clone(),
        };
        let text = ron::ser::to_string_pretty(&store, ron::ser::PrettyConfig::default())
            .map_err(|error| format!("serialize: {error}"))?;

        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }

        // Write to a temporary file first so a crash cannot truncate the data.
        let temporary = self.path.with_extension("ron.tmp");
        fs::write(&temporary, &text)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        match fs::rename(&temporary, &self.path) {
            Ok(()) => Ok(()),
            Err(_) => {
                // `rename` cannot replace a file on some platforms.
                fs::write(&self.path, text)
                    .map_err(|error| format!("write {}: {error}", self.path.display()))?;
                let _ = fs::remove_file(&temporary);
                Ok(())
            }
        }
    }

    fn sort_by_name(&mut self) {
        self.histories.sort_by(|a, b| {
            a.name()
                .to_lowercase()
                .cmp(&b.name().to_lowercase())
                .then_with(|| a.name().cmp(b.name()))
        });
    }

    /// Drop duplicate names and fix review order. Returns how many items were dropped.
    fn sanitize(&mut self) -> usize {
        let before = self.histories.len();
        let mut seen = HashSet::new();
        self.histories
            .retain(|history| seen.insert(history.name().to_string()));
        for history in &mut self.histories {
            history.sort_reviews();
        }
        self.sort_by_name();
        before - self.histories.len()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn manager() -> Manager {
        Manager::new(PathBuf::from("unused-test.ron"))
    }

    fn temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("srs-{tag}-{}-{nanos}.ron", std::process::id()))
    }

    #[test]
    fn add_rejects_duplicates_and_empty_names() {
        let mut manager = manager();
        assert!(manager.add("kanji").is_ok());
        assert!(matches!(manager.add("kanji"), Err(Error::Duplicate(_))));
        assert!(matches!(manager.add("   "), Err(Error::EmptyName)));
        assert_eq!(manager.histories().len(), 1);
    }

    #[test]
    fn review_builds_a_schedule() {
        let mut manager = manager();
        manager.add("kanji").unwrap();
        let now = Utc::now();
        manager.review("kanji", Rating::Good, now).unwrap();

        let summary = manager.summary("kanji").unwrap();
        assert_eq!(summary.reviews, 1);
        assert!(summary.memory.is_some());
        assert!(summary.memory.unwrap().stability > 0.0);
        assert!(summary.due > now);
    }

    #[test]
    fn editing_a_review_replays_later_reviews() {
        let mut manager = manager();
        manager.add("kanji").unwrap();
        let now = Utc::now();
        manager.review("kanji", Rating::Good, now).unwrap();
        manager
            .review("kanji", Rating::Good, now + Duration::days(5))
            .unwrap();
        manager
            .review("kanji", Rating::Easy, now + Duration::days(15))
            .unwrap();

        let before = manager.summary("kanji").unwrap();
        manager.set_rating("kanji", 0, Rating::Again).unwrap();
        let after = manager.summary("kanji").unwrap();

        assert_eq!(after.reviews, 3);
        assert_ne!(
            before.memory.unwrap().stability,
            after.memory.unwrap().stability
        );
        assert_ne!(before.due, after.due);
    }

    #[test]
    fn deleting_reviews_keeps_the_rest() {
        let mut manager = manager();
        manager.add("kanji").unwrap();
        let now = Utc::now();
        for days in [0, 5, 10] {
            manager
                .review("kanji", Rating::Good, now + Duration::days(days))
                .unwrap();
        }

        manager.delete_review("kanji", 1).unwrap();
        let history = manager.get("kanji").unwrap();
        assert_eq!(history.reviews().len(), 2);
        assert_eq!(history.reviews()[0].when, now);
        assert_eq!(history.reviews()[1].when, now + Duration::days(10));

        manager.undo_last("kanji").unwrap();
        assert_eq!(manager.get("kanji").unwrap().reviews().len(), 1);
        manager.undo_last("kanji").unwrap();
        assert!(manager.get("kanji").unwrap().reviews().is_empty());
        assert!(matches!(manager.undo_last("kanji"), Err(Error::NotFound)));
    }

    #[test]
    fn review_times_must_stay_ordered() {
        let mut manager = manager();
        manager.add("kanji").unwrap();
        let now = Utc::now();
        manager.review("kanji", Rating::Good, now).unwrap();
        manager
            .review("kanji", Rating::Good, now + Duration::days(5))
            .unwrap();

        assert!(matches!(
            manager.set_review_time("kanji", 0, now + Duration::days(10)),
            Err(Error::InvalidTime)
        ));
        assert!(matches!(
            manager.review("kanji", Rating::Good, now - Duration::days(1)),
            Err(Error::InvalidTime)
        ));
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_path("round-trip");
        let mut manager = Manager::new(path.clone());
        manager.add("kanji").unwrap();
        manager.add("vocab").unwrap();
        let now = Utc::now();
        manager.review("kanji", Rating::Good, now).unwrap();
        manager
            .review("kanji", Rating::Hard, now + Duration::days(3))
            .unwrap();
        manager.save().unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("kanji"));
        assert!(text.contains("Good"));

        let (loaded, notice) = Manager::load(path.clone());
        assert!(notice.is_none());
        assert_eq!(loaded.histories().len(), 2);
        let history = loaded.get("kanji").unwrap();
        assert_eq!(history.reviews().len(), 2);
        assert_eq!(history.reviews()[0].rating, Rating::Good);
        assert_eq!(history.reviews()[1].rating, Rating::Hard);
        assert_eq!(history.reviews()[0].when, now);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_file_starts_empty() {
        let path = temp_path("missing");
        let (manager, notice) = Manager::load(path);
        assert!(manager.histories().is_empty());
        assert!(notice.is_some_and(|notice| !notice.error));
    }
}
