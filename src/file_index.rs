use filetime::FileTime;
use regex::Regex;
use std::io;
use std::path::{Path, PathBuf};
use std::fs::File;
use std::cmp::Ordering;
use std::collections::hash_map;
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use chrono::{NaiveDate, NaiveTime, NaiveDateTime, Utc};

const TAG_NAME: &str = ".waa";
const MAX_DBS: usize = 10;

#[derive(Debug,Error)]
pub enum FileIndexError {
    #[error(display = "An IO error occurred involving {:?}: {}", _1, _0)]
    Io(io::Error, PathBuf),

    #[error(display = "An IO error occurred while copying: {}\nSource: {:?}\nTarget:{:?}", _0, _1, _2)]
    Cp(io::Error, PathBuf, PathBuf),

    #[error(display = "The supplied folder was not a WhatsApp folder: {:?}", _0)]
    NotWhatsAppFolder(PathBuf),

    #[error(display = "The supplied folder was not an archive folder but not empty: {:?}", _0)]
    NewArchiveFolderNotEmpty(PathBuf),

    #[error(display = "After a copy operation, the metadata of the two files did not match:\nSource: {:?}\nTarget: {:?}", _0, _1)]
    FileMismatch(PathBuf, PathBuf),

    #[error(display = "A file was unexpectedly missing: {:?}", _0)]
    FileMissing(PathBuf),
}

impl<P: AsRef<Path>> From<(io::Error, P)> for FileIndexError {
    fn from(err: (io::Error, P)) -> Self {
        FileIndexError::Io(err.0, err.1.as_ref().to_owned())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    modification_time: FileTime,
    estimated_creation_date: NaiveDateTime,
    size: u64,
}

#[derive(Debug)]
pub enum IndexType {
    Original,
    Archive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionType {
    Real,
    Dry,
}

#[derive(Clone, Copy, Debug)]
pub enum FileOrder {
    Largest,
    Oldest,
    LargestOldest,
}

#[derive(Clone, Copy, Debug)]
pub enum ParseFileOrderError {
    UnknownOrder,
}

impl FromStr for FileOrder {
    type Err = ParseFileOrderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_string();
        match s.as_ref() {
            "largest" => Ok(FileOrder::Largest),
            "oldest" => Ok(FileOrder::Oldest),
            "largest_oldest" => Ok(FileOrder::LargestOldest),
            _ => Err(ParseFileOrderError::UnknownOrder),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum DataLimit {
    Infinite,
    Bytes(u64),
}

impl DataLimit {
    pub fn from_bytes(count: u64) -> DataLimit {
        DataLimit::Bytes(count)
    }
}

impl FileOrder {
    pub fn compare(&self, left: &FileInfo, right: &FileInfo) -> Ordering {
        match *self {
            FileOrder::Largest => left.size.cmp(&right.size).reverse(),
            FileOrder::Oldest => left.estimate_creation_date().cmp(&right.estimate_creation_date()),
            FileOrder::LargestOldest => {
                let now = Utc::now().naive_utc();
                let left_offset = now.signed_duration_since(left.estimate_creation_date());
                let right_offset = now.signed_duration_since(right.estimate_creation_date());
                let left_val = Self::evaluate_largest_oldest(left.size, left_offset.num_milliseconds() as f64);
                let right_val = Self::evaluate_largest_oldest(right.size, right_offset.num_milliseconds() as f64);
                left_val.partial_cmp(&right_val).unwrap().reverse()
            },
        }
    }

    fn evaluate_largest_oldest(size: u64, age_ms: f64) -> f64 {
        let age_days = age_ms / (1000.0 * 60.0 * 60.0 * 24.0);
        let half_life_days = 30.4375;
        (size as f64) * 2.0_f64.powf(age_days / half_life_days)
    }
}

#[derive(Debug)]
pub enum FileFilter {
    All,
    MinAgeDays(u32)
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

#[derive(Debug)]
pub struct FileQuery {
    order: FileOrder,
    limit: DataLimit,
    filter: FileFilter,
}

impl FileQuery {
    pub fn new() -> FileQuery {
        FileQuery {
            order: FileOrder::Oldest,
            limit: DataLimit::Infinite,
            filter: FileFilter::All,
        }
    }

    pub fn set_order(&mut self, order: FileOrder) {
        self.order = order;
    }

    pub fn set_limit(&mut self, limit: DataLimit) {
        self.limit = limit;
    }

    pub fn set_filter(&mut self, filter: FileFilter) {
        self.filter = filter;
    }
}

impl FileInfo {
    fn new(path: &Path) -> Result<FileInfo, FileIndexError> {
        let filename = path.file_name().unwrap();
        let metadata = path.metadata().map_err(|e| (e, path))?;
        let modification_time = FileTime::from_last_modification_time(&metadata);
        let estimated_creation_date = Self::creation_date_from_name(filename.as_ref())
            .unwrap_or(NaiveDateTime::from_timestamp(modification_time.unix_seconds(),
                                                     modification_time.nanoseconds()));
        let result = FileInfo {
            modification_time,
            estimated_creation_date,
            size: metadata.len(),
        };
        Ok(result)
    }

    fn set_modification_time(&self, path: &Path) -> Result<(), FileIndexError> {
        let file = File::open(&path).map_err(|e| (e, path))?;
        let result = filetime::set_file_handle_times(&file, None, Some(self.modification_time))
            .map_err(|e| (e, path))?;
        Ok(result)
    }

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

    fn estimate_creation_date(&self) -> NaiveDateTime {
        self.estimated_creation_date
    }
}

#[derive(Debug)]
pub struct FileIndex {
    index_type: IndexType,
    action_type: ActionType,
    path: PathBuf,
    entries: HashMap<PathBuf, FileInfo>,
}

impl FileIndex {
    pub fn new<P: AsRef<Path>>(index_type: IndexType, path: P, action_type: ActionType) -> Result<FileIndex, FileIndexError> {
        let path = path.as_ref();
        let mut new = false;
        match index_type {
            IndexType::Original => {
                let db_path = path.join("Databases").join("msgstore.db.crypt12");
                let tag_path = path.join(TAG_NAME);
                if !db_path.exists() || tag_path.exists() {
                    return Err(FileIndexError::NotWhatsAppFolder(path.to_owned()));
                }
            },
            IndexType::Archive => {
                if !path.exists() {
                    if action_type == ActionType::Real {
                        std::fs::create_dir_all(path).map_err(|e| (e, path))?;
                    }
                }
                let tag_path = path.join(TAG_NAME);
                if !tag_path.exists() {
                    if action_type == ActionType::Real {
                        let num_entries = path.read_dir().map_err(|e| (e, path))?.count();
                        if num_entries == 0 {
                            std::fs::write(&tag_path, &[]).map_err(|e| (e, &tag_path))?;
                        } else {
                            return Err(FileIndexError::NewArchiveFolderNotEmpty(path.to_owned()));
                        }
                    } else {
                        new = true;
                    }
                }
            },
        };
        let path = if action_type == ActionType::Real {
            path.canonicalize().map_err(|e| (e, path))?
        } else {
            if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
                let parent = parent.canonicalize().map_err(|e| (e, parent))?;
                parent.join(file_name).to_path_buf()
            } else {
                path.to_path_buf()
            }
        };
        let mut result = FileIndex {
            index_type,
            path: path.to_owned(),
            entries: HashMap::new(),
            action_type,
        };
        // So that dry-run mode doesn't error when a new folder hasn't been created
        if !new { result.rebuild_index()?; }
        Ok(result)
    }

    fn get_relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.path)
            .expect("Unable to strip prefix").to_owned()
    }

    fn rebuild_index(&mut self) -> Result<(), FileIndexError> {
        let mut remaining = VecDeque::new();
        remaining.push_back(self.path.clone());
        self.entries.clear();
        while let Some(path) = remaining.pop_front() {
            for entry in path.read_dir().map_err(|e| (e, &path))? {
                let entry = entry.map_err(|e| (e, &path))?;
                if entry.path().file_name().map(|n| n == TAG_NAME).unwrap_or(false) {
                    continue;
                }
                let ftype = entry.file_type().map_err(|e| (e, entry.path()))?;
                if ftype.is_file() {
                    let path = entry.path();
                    let info = FileInfo::new(&path)?;
                    let rel_path = self.get_relative_path(&path);
                    self.entries.insert(rel_path, info);
                } else if ftype.is_dir() {
                    remaining.push_back(entry.path());
                } else {
                    warn!("Ignoring unexpected directory entry: {:?}", entry);
                }
            }
        }
        Ok(())
    }

    fn import_file_maybe_metadata(&mut self, relative_path: &Path, source: &Path, info: Option<&FileInfo>) -> Result<(), FileIndexError> {
        let dest_path = self.path.join(relative_path);
        let mut do_copy = || {
            assert!(relative_path.is_relative());
            if self.action_type == ActionType::Real {
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| (e, parent))?;
                }
                std::fs::copy(source, &dest_path).map_err(|e| FileIndexError::Cp(e, source.to_owned(), dest_path.to_owned()))?;
                match info {
                    None => Ok(()),
                    Some(info) => {
                        info.set_modification_time(&dest_path)?;
                        let actual_metadata = FileInfo::new(&dest_path)?;
                        if actual_metadata == *info {
                            self.entries.insert(relative_path.to_path_buf(), actual_metadata);
                            Ok(())
                        } else {
                            Err(FileIndexError::FileMismatch(source.to_owned(), dest_path.to_owned()))
                        }
                    },
                }
            } else {
                let actual_metadata = FileInfo::new(source)?;
                self.entries.insert(relative_path.to_path_buf(), actual_metadata);
                Ok(())
            }
        };
        match do_copy() {
            Ok(()) => Ok(()),
            Err(e) => {
                if self.action_type == ActionType::Real {
                    let _ = std::fs::remove_file(&dest_path)
                        .map_err(|e| eprintln!("Additional error during delete of incompletely copied file: {:?}", e));
                }
                Err(e)
            },
        }
    }

    pub fn import_file(&mut self, relative_path: &Path, source: &Path) -> Result<(), FileIndexError> {
        self.import_file_maybe_metadata(relative_path, source, None)
    }


    pub fn import_file_with_metadata(&mut self, relative_path: &Path, source: &Path, info: &FileInfo) -> Result<(), FileIndexError> {
        self.import_file_maybe_metadata(relative_path, source, Some(info))
    }

    pub fn remove_file(&mut self, path: &Path) -> Result<(), FileIndexError> {
        if let hash_map::Entry::Occupied(entry) = self.entries.entry(path.to_path_buf()) {
            let path = self.path.join(path);
            println!("Deleting {}", path.to_string_lossy());
            if self.action_type == ActionType::Real {
                std::fs::remove_file(&path).map_err(|e| (e, path))?;
            }
            entry.remove_entry();
            Ok(())
        } else {
            Err(FileIndexError::FileMissing(path.to_owned()))
        }
    }

    pub fn clean_old_dbs(&mut self) -> Result<(), FileIndexError> {
        let date_regex = Regex::new(r"....-..-..").unwrap();
        let mut paths: Vec<PathBuf> = self.entries.iter()
            .map(|(rel_path, _)| rel_path.to_owned())
            .filter(|p| p.starts_with("Databases"))
            .filter(|p| date_regex.is_match(p.to_string_lossy().as_ref()))
            .collect();
        if paths.len() <= MAX_DBS { return Ok(()); }
        paths.sort();
        let delete_count = paths.len() - MAX_DBS;
        let to_delete = &paths[..delete_count];
        for db in to_delete { self.remove_file(db)?; }
        Ok(())
    }

    pub fn mirror_from(&mut self, source_index: &FileIndex) -> Result<(), FileIndexError> {
        let source = &source_index.entries;
        // Check common files match in terms of metadata
        {
            let mut common = self.entries.clone();
            common.retain(|k, _| source.contains_key(k.as_path()));
            for (rel_path, value) in &common {
                let other = source.get(rel_path).unwrap();
                if value != other {
                    println!("Updating changed file {:?}", rel_path);
                    let source_path = source_index.path.join(rel_path);
                    self.import_file_with_metadata(rel_path, &source_path, other)?;
                }
            }
        }

        // Copy missing files to archive
        {
            let mut missing = source.clone();
            missing.retain(|k, _| !self.entries.contains_key(k.as_path()));
            for (rel_path, value) in &missing {
                println!("Copying missing file: {:?}", rel_path);
                let source_path = source_index.path.join(rel_path);
                self.import_file_with_metadata(rel_path, &source_path, value)?;
            }
        }

        self.clean_old_dbs()
    }

    pub fn get_size_bytes(&self) -> u64 {
        self.entries.iter().map(|(_, fi)| fi.size).fold(0, |a, b| a + b)
    }

    pub fn get_deletion_candidates(&self, query: &FileQuery) -> Vec<(PathBuf, FileInfo)> {
        let mut media_entries: Vec<(PathBuf, FileInfo)> = self.entries.iter()
                .filter(|(p, _)| p.starts_with("Media"))
                .filter(|(p, _)| p.file_name().map(|e| e != ".nomedia").unwrap_or(true))
                .filter(|(_, i)| query.filter.matches(i))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
        media_entries.sort_unstable_by(|(_, a), (_, b)| query.order.compare(a, b));
        match query.limit {
            DataLimit::Infinite => { media_entries.clear(); },
            DataLimit::Bytes(limit) => {
                let mut total: u64 = self.get_size_bytes();
                let mut count = 0;
                for (idx, (_, entry)) in media_entries.iter().enumerate() {
                    count = idx;
                    if total <= limit {
                        break;
                    }
                    total = total.saturating_sub(entry.size);
                }
                media_entries.truncate(count);
            },
        }
        media_entries
    }

    pub fn filter_existing(&self, list: &Vec<PathBuf>) -> Vec<PathBuf> {
        list.iter().filter(|p| self.entries.contains_key(p.as_path())).cloned().collect()
    }

    pub fn remove_files<I: IntoIterator<Item = impl AsRef<Path>>>(&mut self, files: I) -> Result<(), FileIndexError> {
        for file in files { self.remove_file(file.as_ref())?; }
        Ok(())
    }
}
