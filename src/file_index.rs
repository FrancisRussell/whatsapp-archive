use filetime::FileTime;
use regex::Regex;
use std::io;
use std::path::{Path, PathBuf};
use std::fs::{DirEntry, File};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use chrono::{NaiveDate, NaiveTime, NaiveDateTime, Utc};

const TAG_NAME: &str = ".waa";
const MAX_DBS: usize = 10;

#[derive(Debug,Error)]
pub enum FileIndexError {
    Io(io::Error),
    NotWhatsAppFolder,
    NotArchiveFolder,
    NewArchiveFolderNotEmpty,
    FileMismatch,
    FileMissing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    relative_path: PathBuf,
    modification_time: FileTime,
    estimated_creation_date: NaiveDateTime,
    size: u64,
}

#[derive(Debug)]
pub enum IndexType {
    Original,
    Archive,
}

#[derive(Clone, Copy, Debug)]
pub enum FileOrder {
    Largest,
    Oldest,
    LargestOldest,
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
                let left_val = (left.size as f64) * (left_offset.num_milliseconds() as f64);
                let right_val = (right.size as f64) * (right_offset.num_milliseconds() as f64);
                left_val.partial_cmp(&right_val).unwrap().reverse()
            },
        }
    }
}

#[derive(Debug)]
pub struct FileQuery {
    order: FileOrder,
    limit: DataLimit,
}

impl FileQuery {
    pub fn new() -> FileQuery {
        FileQuery {
            order: FileOrder::Oldest,
            limit: DataLimit::Infinite,
        }
    }

    pub fn set_order(&mut self, order: FileOrder) {
        self.order = order;
    }

    pub fn set_limit(&mut self, limit: DataLimit) {
        self.limit = limit;
    }
}

impl FileInfo {
    fn new<P: AsRef<Path>>(root: P, entry: DirEntry) -> Result<FileInfo, FileIndexError> {
        let relative_path = entry.path().strip_prefix(root.as_ref()).expect("Unable to strip prefix").to_owned();
        let metadata = entry.metadata()?;
        let modification_time = FileTime::from_last_modification_time(&metadata);
        let estimated_creation_date = Self::creation_date_from_name(&relative_path)
            .unwrap_or(NaiveDateTime::from_timestamp(modification_time.unix_seconds(),
                                                     modification_time.nanoseconds()));
        let result = FileInfo {
            relative_path,
            modification_time,
            estimated_creation_date,
            size: metadata.len(),
        };
        Ok(result)
    }

    fn set_modification_time(&self, file: &File) -> Result<(), FileIndexError> {
        Ok(filetime::set_file_handle_times(file, None, Some(self.modification_time))?)
    }

    fn creation_date_from_name(relative_path: &Path) -> Option<NaiveDateTime> {
        let day_regex = Regex::new(r"^.*-(\d{8})-WA[0-9]{4}\..+$").unwrap();
        let file_name = relative_path.file_name().unwrap().to_string_lossy().as_ref().to_string();
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
    path: PathBuf,
    entries: Vec<FileInfo>,
}

impl FileIndex {
    pub fn new<P: AsRef<Path>>(index_type: IndexType, path: P) -> Result<FileIndex, FileIndexError> {
        let path = path.as_ref();
        match index_type {
            IndexType::Original => {
                let db_path = path.join("Databases").join("msgstore.db.crypt12");
                let tag_path = path.join(TAG_NAME);
                if !db_path.exists() || tag_path.exists() {
                    return Err(FileIndexError::NotWhatsAppFolder);
                }
            },
            IndexType::Archive => {
                if !path.exists() {
                    std::fs::create_dir_all(path)?;
                }
                let tag_path = path.join(TAG_NAME);
                if !tag_path.exists() {
                    let num_entries = path.read_dir()?.count();
                    if num_entries == 0 {
                        std::fs::write(tag_path, &[])?;
                    } else {
                        return Err(FileIndexError::NewArchiveFolderNotEmpty);
                    }
                }
            },
        };
        let path = path.canonicalize()?;
        let entries = Self::build_index(&path)?;
        let result = FileIndex {
            index_type,
            path: path.to_owned(),
            entries,
        };
        Ok(result)
    }

    fn build_index(dir: &Path) -> Result<Vec<FileInfo>, FileIndexError> {
        let mut result = Vec::new();
        let mut remaining = VecDeque::new();
        remaining.push_back(dir.to_owned());
        while let Some(path) = remaining.pop_front() {
            for entry in path.read_dir()? {
                let entry = entry?;
                if entry.path().file_name().map(|n| n == TAG_NAME).unwrap_or(false) {
                    continue;
                }
                let ftype = entry.file_type()?;
                if ftype.is_file() {
                    result.push(FileInfo::new(dir, entry)?);
                } else if ftype.is_dir() {
                    remaining.push_back(entry.path());
                } else {
                    warn!("Ignoring unexpected directory entry: {:?}", entry);
                }
            }
        }
        Ok(result)
    }

    fn index_as_hash(&self) -> HashMap<PathBuf, FileInfo> {
        let mut result = HashMap::new();
        for entry in &self.entries {
            result.insert(entry.relative_path.to_owned(), entry.clone());
        }
        result
    }

    fn copy_file<P1: AsRef<Path>, P2: AsRef<Path>>(from_root: P1, to_root: P2, file_info: &FileInfo) -> Result<(), FileIndexError> {
        let rel_path = &file_info.relative_path;
        let source_path = from_root.as_ref().join(&rel_path);
        let dest_path = to_root.as_ref().join(&rel_path);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source_path, &dest_path)?;
        let copied = File::open(&dest_path)?;
        file_info.set_modification_time(&copied)?;
        Ok(())
    }

    pub fn clean_old_dbs(&mut self) -> Result<(), FileIndexError> {
        let date_regex = Regex::new(r"....-..-..").unwrap();
        let mut paths: Vec<PathBuf> = self.entries.iter()
            .map(|fi| fi.relative_path.to_owned())
            .filter(|p| p.starts_with("Databases"))
            .filter(|p| date_regex.is_match(p.to_string_lossy().as_ref()))
            .collect();
        if paths.len() <= MAX_DBS { return Ok(()); }
        paths.sort();
        let delete_count = paths.len() - MAX_DBS;
        let to_delete = &paths[..delete_count];
        for db in to_delete {
            let path = self.path.join(db);
            println!("Deleting old message database: {:?}", path);
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn mirror_from(&mut self, source_index: &FileIndex) -> Result<(), FileIndexError> {
        let source = source_index.index_as_hash();
        let dest = self.index_as_hash();

        // Check common files match in terms of metadata
        {
            let mut common = dest.clone();
            common.retain(|k, _| source.contains_key(k.as_path()));
            for (rel_path, value) in &common {
                let other = source.get(rel_path).unwrap();
                if value != other {
                    println!("Copying file with metadata mismatch {:?}", rel_path);
                    if let Err(e) = Self::copy_file(&source_index.path, &self.path, other) {
                        let dest_path = self.path.join(rel_path);
                        let _ = std::fs::remove_file(&dest_path)
                            .map_err(|e| eprintln!("During delete of incompletely copied file: {:?}", e));
                        return Err(e)
                    }
                }
            }
        }

        // Copy missing files to archive
        {
            let mut missing = source.clone();
            missing.retain(|k, _| !dest.contains_key(k.as_path()));
            for (rel_path, value) in &missing {
                println!("Copying missing file: {:?}", rel_path);
                if let Err(e) = Self::copy_file(&source_index.path, &self.path, value) {
                    let dest_path = self.path.join(rel_path);
                    let _ = std::fs::remove_file(&dest_path)
                        .map_err(|e| eprintln!("During delete of incompletely copied file: {:?}", e));
                    return Err(e)
                }
            }
        }

        self.clean_old_dbs()?;
        self.reindex()
    }

    pub fn get_size_bytes(&self) -> u64 {
        self.entries.iter().map(|fi| fi.size).fold(0, |a, b| a + b)
    }

    pub fn get_deletion_candidates(&self, query: &FileQuery) -> Vec<FileInfo> {
        let mut media_entries: Vec<FileInfo> = self.entries.iter()
                .filter(|p| p.relative_path.starts_with("Media"))
                .filter(|e| e.relative_path.file_name().map(|e| e != ".nomedia").unwrap_or(true))
                .cloned()
                .collect();
        media_entries.sort_unstable_by(|a, b| query.order.compare(a, b));
        match query.limit {
            DataLimit::Infinite => { media_entries.clear(); },
            DataLimit::Bytes(limit) => {
                let mut total: u64 = self.get_size_bytes();
                let mut count = 0;
                for (idx, entry) in media_entries.iter().enumerate() {
                    if total <= limit {
                        count = idx;
                        break;
                    }
                    total = total.saturating_sub(entry.size);
                }
                media_entries.truncate(count);
            },
        }
        media_entries
    }

    pub fn filter_existing(&self, list: &Vec<FileInfo>) -> Vec<FileInfo> {
        let index = self.index_as_hash();
        list.iter().filter(|p| index.contains_key(p.relative_path.as_path())).cloned().collect()
    }

    fn reindex(&mut self) -> Result<(), FileIndexError> {
        let new_entries = Self::build_index(&self.path)?;
        self.entries = new_entries;
        Ok(())
    }

    pub fn delete_files_from_infos<'a, I: IntoIterator<Item = &'a FileInfo>>(&mut self, infos: I) -> Result<(), FileIndexError> {
        let index = self.index_as_hash();
        for other_info in infos {
            if let Some(info) = index.get(other_info.relative_path.as_path()) {
                if info != other_info {
                    return Err(FileIndexError::FileMismatch);
                }
                let path = self.path.join(&info.relative_path);
                println!("Deleting {}", path.to_string_lossy());
                std::fs::remove_file(&path)?;
            } else {
                return Err(FileIndexError::FileMissing);
            }
        }
        self.reindex()
    }
}
