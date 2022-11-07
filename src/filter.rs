use std::str::FromStr;

use chrono::Utc;

use crate::FileInfo;

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
    pub fn set_order(&mut self, order: FileScore) { self.order = order; }

    pub fn set_limit(&mut self, limit: DataLimit) { self.limit = limit; }

    pub fn set_filter(&mut self, filter: FileFilter) { self.filter = filter; }
}

impl FromStr for FileScore {
    type Err = ParseFileScoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_string();
        match s.as_ref() {
            "largest" => Ok(FileScore::Largest),
            "oldest" => Ok(FileScore::Oldest),
            "largest_oldest" => Ok(FileScore::LargestOldest),
            _ => Err(ParseFileScoreError::UnknownOrder),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum FileScore {
    Largest,
    Oldest,
    LargestOldest,
}

#[derive(Clone, Copy, Debug)]
pub enum ParseFileScoreError {
    UnknownOrder,
}

impl FileScore {
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

#[derive(Clone, Copy, Debug)]
pub enum DataLimit {
    Infinite,
    Bytes(u64),
}

impl DataLimit {
    pub fn from_bytes(count: u64) -> DataLimit { DataLimit::Bytes(count) }
}

#[derive(Debug)]
pub enum FileFilter {
    All,
    MinAgeDays(u32),
}

impl FileFilter {
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
