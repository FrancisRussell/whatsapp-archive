use std::fs::File;
use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use filetime::FileTime;
use regex::Regex;

use crate::FileIndexError;

/// Represents file metadata
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    modification_time: FileTime,
    estimated_creation_date: NaiveDateTime,
    size: u64,
}

impl FileInfo {
    /// Constructs a new `FileInfo` representing the metadata of the specified
    /// file
    pub fn new(path: &Path) -> Result<FileInfo, FileIndexError> {
        let filename = path.file_name().expect("Unable to get filename from path");
        let metadata = path.metadata().map_err(|e| (e, path))?;
        let modification_time = FileTime::from_last_modification_time(&metadata);
        let estimated_creation_date = Self::creation_date_from_name(filename.as_ref()).unwrap_or_else(|| {
            NaiveDateTime::from_timestamp(modification_time.unix_seconds(), modification_time.nanoseconds())
        });
        let result = FileInfo { modification_time, estimated_creation_date, size: metadata.len() };
        Ok(result)
    }

    /// Alters the modification time of the file at `path` to the one stored in
    /// the `FileInfo`.
    pub fn set_modification_time(&self, path: &Path) -> Result<(), FileIndexError> {
        let file = File::open(path).map_err(|e| (e, path))?;
        filetime::set_file_handle_times(&file, None, Some(self.modification_time)).map_err(|e| (e, path))?;
        Ok(())
    }

    /// Attempts to estimate the creation date of a file based on WhatsApp's
    /// media file naming convention
    fn creation_date_from_name(filename: &Path) -> Option<NaiveDateTime> {
        let day_regex = Regex::new(r"^.*-(\d{8})-WA[0-9]{4}\..+$").unwrap();
        let file_name = filename.to_string_lossy();
        if let Some(capture) = day_regex.captures(&file_name).and_then(|c| c.get(1)) {
            let date_time = NaiveDate::parse_from_str(capture.as_str(), "%Y%m%d")
                .map(|date| NaiveDateTime::new(date, NaiveTime::from_hms(0, 0, 0)));
            date_time.ok()
        } else {
            None
        }
    }

    /// Estimate when this file was created. This will attempt to infer the
    /// creation time from WhatsApp's naming convention, otherwise will use
    /// the filesystem metadata.
    pub fn estimate_creation_date(&self) -> NaiveDateTime { self.estimated_creation_date }

    /// The size of the file in bytes
    pub fn get_size(&self) -> u64 { self.size }
}
