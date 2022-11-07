use std::str::FromStr;

use chrono::Utc;

use crate::FileInfo;

/// A query for files
#[derive(Debug)]
pub struct FileQuery {
    pub(crate) order: FileScore,
    pub(crate) data_limit: DataLimit,
    pub(crate) priority: FilePredicate,
}

impl Default for FileQuery {
    fn default() -> FileQuery {
        FileQuery { order: FileScore::Newer, data_limit: DataLimit::Infinite, priority: FilePredicate::none() }
    }
}

impl FileQuery {
    /// Sets the scoring function used to order files
    pub fn set_order(&mut self, order: FileScore) { self.order = order; }

    /// Sets the maximum storage used by the returned files
    pub fn set_limit(&mut self, data_limit: DataLimit) { self.data_limit = data_limit; }

    /// Sets a predicate for high-priority files
    pub fn set_priority(&mut self, predicate: FilePredicate) { self.priority = predicate; }
}

impl FromStr for FileScore {
    type Err = ParseFileScoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s {
            "smaller" => Ok(FileScore::Smaller),
            "newer" => Ok(FileScore::Newer),
            "smaller_newer" => Ok(FileScore::SmallerNewer),
            _ => Err(ParseFileScoreError::UnknownOrder),
        }
    }
}

/// Ranking function for files
#[derive(Clone, Copy, Debug)]
pub enum FileScore {
    /// Score is negatively proportional to file size
    Smaller,

    /// Score is negatively proportional to file age
    Newer,

    /// Score decreases proportionally with size and exponentially with age
    SmallerNewer,
}

/// Error type for parsing file ordering
#[derive(Clone, Copy, Debug)]
pub enum ParseFileScoreError {
    UnknownOrder,
}

impl FileScore {
    /// Evaluates the score for a file (smaller is more important)
    pub fn evaluate(&self, info: &FileInfo) -> f64 {
        match *self {
            FileScore::Smaller => -(info.get_size() as f64),
            FileScore::Newer => -(info.estimate_creation_date().timestamp_millis() as f64),
            FileScore::SmallerNewer => {
                let now = Utc::now().naive_utc();
                let offset = now.signed_duration_since(info.estimate_creation_date());
                Self::evaluate_smaller_newer(info.get_size(), offset.num_milliseconds() as f64)
            }
        }
    }

    fn evaluate_smaller_newer(size: u64, age_ms: f64) -> f64 {
        let age_days = age_ms / (1000.0 * 60.0 * 60.0 * 24.0);
        let half_life_days = 30.4375;
        -(size as f64) * 2.0_f64.powf(age_days / half_life_days)
    }
}

/// A limit for the amout of data consumed
#[derive(Clone, Copy, Debug)]
pub enum DataLimit {
    /// No limit
    Infinite,

    /// A byte count
    Bytes(u64),
}

impl DataLimit {
    /// Constructs a `DataLimit` from a byte count
    pub fn from_bytes(count: u64) -> DataLimit { DataLimit::Bytes(count) }
}

/// A predicate for files
#[derive(Debug)]
pub enum FilePredicate {
    /// Always returns the specified `bool`
    Constant(bool),

    /// Only files younger or equal to the specified duration
    AgeLessThan(chrono::Duration),
}

impl FilePredicate {
    /// Returns `true` for any file
    pub fn all() -> FilePredicate { FilePredicate::Constant(true) }

    /// Returns `false` for any file
    pub fn none() -> FilePredicate { FilePredicate::Constant(false) }

    pub fn matches(&self, file_info: &FileInfo) -> bool {
        match self {
            FilePredicate::Constant(b) => *b,
            FilePredicate::AgeLessThan(max) => {
                let now = Utc::now().naive_utc();
                let age = now.signed_duration_since(file_info.estimate_creation_date());
                age <= *max
            }
        }
    }
}
