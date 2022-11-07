use std::str::FromStr;

use chrono::Utc;

use crate::FileInfo;

/// A query for files
#[derive(Debug)]
pub struct FileQuery {
    pub(crate) order: FileScore,
    pub(crate) limit: DataLimit,
    pub(crate) filter: FileFilter,
}

impl Default for FileQuery {
    fn default() -> FileQuery {
        FileQuery { order: FileScore::Oldest, limit: DataLimit::Infinite, filter: FileFilter::All }
    }
}

impl FileQuery {
    /// Sets the scoring function used to order files
    pub fn set_order(&mut self, order: FileScore) { self.order = order; }

    /// Sets the maximum storage used by the returned files
    pub fn set_limit(&mut self, limit: DataLimit) { self.limit = limit; }

    /// Sets a filter for excluding files
    pub fn set_filter(&mut self, filter: FileFilter) { self.filter = filter; }
}

impl FromStr for FileScore {
    type Err = ParseFileScoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s {
            "largest" => Ok(FileScore::Largest),
            "oldest" => Ok(FileScore::Oldest),
            "largest_oldest" => Ok(FileScore::LargestOldest),
            _ => Err(ParseFileScoreError::UnknownOrder),
        }
    }
}

/// Ranking function for files
#[derive(Clone, Copy, Debug)]
pub enum FileScore {
    /// Score is proportional to file size
    Largest,

    /// Score is proportional to file age
    Oldest,

    /// Score increases proportionally with size and exponentially with age
    LargestOldest,
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
            FileScore::Largest => info.get_size() as f64,
            FileScore::Oldest => info.estimate_creation_date().timestamp_millis() as f64,
            FileScore::LargestOldest => {
                let now = Utc::now().naive_utc();
                let offset = now.signed_duration_since(info.estimate_creation_date());
                Self::evaluate_largest_oldest(info.get_size(), offset.num_milliseconds() as f64)
            }
        }
    }

    fn evaluate_largest_oldest(size: u64, age_ms: f64) -> f64 {
        let age_days = age_ms / (1000.0 * 60.0 * 60.0 * 24.0);
        let half_life_days = 30.4375;
        (size as f64) * 2.0_f64.powf(age_days / half_life_days)
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
pub enum FileFilter {
    /// All files match
    All,

    /// Only files older than the specified number of days match
    MinAgeDays(u32),
}

impl FileFilter {
    /// Returns `true` if the specified `FileInfo` matches the predicate
    pub fn matches(&self, file: &FileInfo) -> bool {
        match *self {
            FileFilter::All => true,
            FileFilter::MinAgeDays(min) => {
                let now = Utc::now().naive_utc();
                let age = now.signed_duration_since(file.estimate_creation_date());
                age.num_days() >= (min as i64)
            }
        }
    }
}
