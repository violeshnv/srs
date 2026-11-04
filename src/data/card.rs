use std::cmp::Ordering;

use chrono::{DateTime, Duration, TimeDelta, Utc};
use fsrs::{FSRS, FSRSError, MemoryState};

pub struct Card {
    pub memeory_state: MemoryState,
    pub review: DateTime<Utc>,
    /// due = review + scheduled_days
    pub due: DateTime<Utc>,
}

impl Card {
    pub fn new(memory_state: Option<MemoryState>) -> Self {
        let review = Utc::now();
        Self {
            review,
            due: review,
            memeory_state: memory_state.unwrap_or(MemoryState {
                stability: 0.0,
                difficulty: 0.0,
            }),
        }
    }

    pub fn scheduled_days(&self) -> f32 {
        (self.due - Utc::now()).num_seconds() as f32 / Duration::days(1).as_seconds_f32()
    }
}

#[derive(Debug)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    InvalidParameter(&'static str),
    Fsrs(FSRSError),
}

impl From<FSRSError> for Error {
    fn from(error: FSRSError) -> Self {
        Self::Fsrs(error)
    }
}

pub struct Item {
    pub card: Card,
    pub rating: Rating,
    /// modified by user
    modified: bool,
}

pub struct History {
    name: String,
    items: Vec<Item>,
}

impl History {
    const DESIRED_RETENTION: f32 = 0.9;

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn items(&self) -> &Vec<Item> {
        &self.items
    }

    pub fn review(
        &mut self,
        fsrs: &FSRS,
        rating: Rating,
        when: DateTime<Utc>,
    ) -> Result<(), Error> {
        let card = {
            let last = self.items.last().unwrap();
            if (when - last.card.review).num_days() < 0 {
                return Err(Error::InvalidParameter(
                    "`when` must be after `last_review`",
                ));
            }
            let elapsed_days = (when - last.card.review).num_days();
            let next_states = fsrs.next_states(
                Some(last.card.memeory_state),
                Self::DESIRED_RETENTION,
                elapsed_days as u32,
            )?;
            let next = match rating {
                Rating::Again => next_states.again,
                Rating::Hard => next_states.hard,
                Rating::Good => next_states.good,
                Rating::Easy => next_states.easy,
            };
            Card {
                memeory_state: next.memory,
                review: when,
                due: when
                    + TimeDelta::seconds(
                        (next.interval * Duration::days(1).as_seconds_f32()) as i64,
                    ),
            }
        };
        self.items.push(Item {
            card,
            rating,
            modified: false,
        });
        Ok(())
    }
}

pub struct Manager {
    fsrs: FSRS,

    histories: Vec<History>,
    order: Box<dyn FnMut(&History, &History) -> Ordering>,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            fsrs: FSRS::default(),
            histories: Vec::new(),
            order: Box::new(|a, b| a.name.cmp(&b.name)),
        }
    }
}

pub enum By<'a> {
    Name(&'a str),
    Index(usize),
}

impl Manager {
    pub fn histories(&self) -> &Vec<History> {
        &self.histories
    }

    fn get_mut_history<'a>(histories: &'a mut Vec<History>, by: By) -> Option<&'a mut History> {
        match by {
            By::Name(name) => histories.iter_mut().find(|h| h.name == name),
            By::Index(index) => histories.get_mut(index),
        }
    }

    pub fn get(&self, by: By) -> Option<&History> {
        match by {
            By::Name(name) => self.histories.iter().find(|h| h.name == name),
            By::Index(index) => self.histories.get(index),
        }
    }

    pub fn get_mut(&mut self, by: By) -> Option<&mut History> {
        Self::get_mut_history(&mut self.histories, by)
    }

    pub fn sort(&mut self, by: impl FnMut(&History, &History) -> Ordering + 'static) {
        self.order = Box::new(by);
        self.histories.sort_by(self.order.as_mut());
    }

    /// Add a new history. Returns an error if a history with the same name already exists.already exists.
    pub fn add(&mut self, name: String) -> Result<usize, usize> {
        let mut new = History {
            name: name,
            items: Vec::new(),
        };
        match self.histories.binary_search_by(|h| (self.order)(h, &new)) {
            Ok(index) => Err(index),
            Err(index) => {
                new.items.push(Item {
                    card: Card::new(None),
                    rating: Rating::Again,
                    modified: false,
                });
                self.histories.insert(index, new);
                Ok(index)
            }
        }
    }

    /// Return the removed items if it exists.
    pub fn remove(&mut self, by: By) -> Option<History> {
        match by {
            By::Name(name) => self.histories.pop_if(|h| h.name == name),
            By::Index(index) => {
                if index < self.histories.len() {
                    Some(self.histories.remove(index))
                } else {
                    None
                }
            }
        }
    }

    /// Review this item and update its memory state and scheduled days.
    pub fn review(&mut self, by: By, rating: Rating, when: DateTime<Utc>) -> Result<(), Error> {
        let history = Self::get_mut_history(&mut self.histories, by).ok_or(Error::NotFound)?;
        history.review(&self.fsrs, rating, when)
    }
}
