use std::borrow::ToOwned;
use std::collections::{hash_map, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use filetime::FileTime;
use log::warn;
use rand::Rng;
use regex::Regex;

use crate::{DataLimit, Error, FileInfo, FileQuery};

const TAG_NAME: &str = ".waa";

/// What the file index is constructed over
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IndexType {
    /// An actual WhatsApp data folder
    Original,

    /// The backup of a WhatsApp data folder
    Archive,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActionType {
    Real,
    Dry,
}

/// A file index for a directory tree
#[derive(Debug)]
pub struct FileIndex {
    _index_type: IndexType,
    action_type: ActionType,
    path: PathBuf,
    entries: HashMap<PathBuf, FileInfo>,
}

#[derive(Debug)]
struct DbInfo {
    pub is_incremental: bool,
    pub file_extension: String,
    pub last_modified: FileTime,
}

impl FileIndex {
    /// Constructs a new index of the files at the specified path.
    pub fn new<P: AsRef<Path>>(index_type: IndexType, path: P, action_type: ActionType) -> Result<FileIndex, Error> {
        let path = path.as_ref();
        let mut new = false;
        match index_type {
            IndexType::Original => {
                let mut found_db = false;
                for suffix in &["crypt14", "crypt15"] {
                    let db_path = path.join("Databases").join(format!("msgstore.db.{}", suffix));
                    if db_path.exists() {
                        found_db = true;
                        break;
                    }
                }
                let tag_path = path.join(TAG_NAME);
                // We check for presence of a DB and that this is not a backup folder
                if !found_db || tag_path.exists() {
                    return Err(Error::NotWhatsAppFolder(path.to_owned()));
                }
            }
            IndexType::Archive => {
                if !path.exists() && action_type == ActionType::Real {
                    std::fs::create_dir_all(path).map_err(|e| (e, path))?;
                }
                let tag_path = path.join(TAG_NAME);
                if !tag_path.exists() {
                    if action_type == ActionType::Real {
                        let num_entries = path.read_dir().map_err(|e| (e, path))?.count();
                        if num_entries == 0 {
                            std::fs::write(&tag_path, []).map_err(|e| (e, &tag_path))?;
                        } else {
                            return Err(Error::NewArchiveFolderNotEmpty(path.to_owned()));
                        }
                    } else {
                        new = true;
                    }
                }
            }
        };
        let path = if action_type == ActionType::Real {
            path.canonicalize().map_err(|e| (e, path))?
        } else if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
            let parent = parent.canonicalize().map_err(|e| (e, parent))?;
            parent.join(file_name)
        } else {
            path.to_path_buf()
        };
        let mut result = FileIndex { _index_type: index_type, path, entries: HashMap::new(), action_type };
        // So that dry-run mode doesn't error when a new folder hasn't been created
        if !new {
            result.rebuild_index()?;
        }
        Ok(result)
    }

    /// Strips the location of the index from an absolute path
    fn get_relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.path).expect("Unable to strip prefix").to_owned()
    }

    /// Traverses the directory structure and builds the index
    fn rebuild_index(&mut self) -> Result<(), Error> {
        let mut remaining = VecDeque::new();
        remaining.push_back(self.path.clone());
        self.entries.clear();
        while let Some(path) = remaining.pop_front() {
            for entry in path.read_dir().map_err(|e| (e, &path))? {
                let entry = entry.map_err(|e| (e, &path))?;
                if entry.path().file_name().map_or(false, |n| n == TAG_NAME) {
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

    /// Attempts to copy a file in a way that minimizes the chance that a
    /// partially written file ends up at the destination path if an IO
    /// error occurs.
    fn safer_copy(source_path: &Path, dest_path: &Path) -> Result<(), Error> {
        let dest_path_temp = {
            let filename = dest_path.file_name().expect("Unable to determine destination filename");
            let parent = dest_path.parent().expect("Unable to determine parent folder of destination file");
            let random: u32 = rand::thread_rng().gen();
            let temp_filename = format!("{}.{:x}.waa.tmp", filename.to_string_lossy(), random);
            parent.join(temp_filename)
        };
        if let Err(e) = std::fs::copy(source_path, &dest_path_temp)
            .map_err(|e| Error::Cp(e, source_path.to_owned(), dest_path_temp.clone()))
            .and_then(|_| {
                let file = std::fs::File::open(&dest_path_temp).map_err(|e| Error::Io(e, dest_path_temp.clone()))?;
                file.sync_data().map_err(|e| Error::Io(e, dest_path_temp.clone()))
            })
            .and_then(|()| {
                std::fs::rename(&dest_path_temp, dest_path)
                    .map_err(|e| Error::Mv(e, dest_path_temp.clone(), dest_path.to_owned()))
            })
        {
            let _ = std::fs::remove_file(dest_path_temp);
            return Err(e);
        }
        Ok(())
    }

    /// Imports the file at `path` into the index at `relative_path` optionally
    /// overriding metadata with the supplied
    fn import_file_maybe_metadata(
        &mut self, relative_path: &Path, source: &Path, info: Option<&FileInfo>,
    ) -> Result<(), Error> {
        let dest_path = self.path.join(relative_path);
        let mut do_copy = || {
            assert!(relative_path.is_relative());
            if self.action_type == ActionType::Real {
                // Create destination folder
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| (e, parent))?;
                }
                Self::safer_copy(source, &dest_path)?;
                match info {
                    None => Ok(()),
                    Some(info) => {
                        // Update modification time on filesystem
                        info.set_modification_time(&dest_path)?;
                        let actual_metadata = FileInfo::new(&dest_path)?;
                        // Check that other metadata matches (e.g. file size)
                        if actual_metadata == *info {
                            self.entries.insert(relative_path.to_path_buf(), actual_metadata);
                            Ok(())
                        } else {
                            Err(Error::FileMismatch(source.to_owned(), dest_path.clone()))
                        }
                    }
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
                    //TODO: no need to error if this file doesn't exist
                    let _ = std::fs::remove_file(&dest_path)
                        .map_err(|e| eprintln!("Additional error during delete of incompletely copied file: {:?}", e));
                }
                Err(e)
            }
        }
    }

    /// Imports the file at `path` into the index at `relative_path`
    pub fn import_file(&mut self, relative_path: &Path, source: &Path) -> Result<(), Error> {
        self.import_file_maybe_metadata(relative_path, source, None)
    }

    /// Imports the file at `path` into the index at `relative_path` with the
    /// supplied metadata.
    pub fn import_file_with_metadata(
        &mut self, relative_path: &Path, source: &Path, info: &FileInfo,
    ) -> Result<(), Error> {
        self.import_file_maybe_metadata(relative_path, source, Some(info))
    }

    /// Removes a file from the index and the filesystem
    pub fn remove_file(&mut self, path: &Path) -> Result<(), Error> {
        if let hash_map::Entry::Occupied(entry) = self.entries.entry(path.to_path_buf()) {
            let path = self.path.join(path);
            println!("Deleting {}", path.to_string_lossy());
            if self.action_type == ActionType::Real {
                std::fs::remove_file(&path).map_err(|e| (e, path))?;
            }
            entry.remove_entry();
            Ok(())
        } else {
            Err(Error::FileMissing(path.to_owned()))
        }
    }

    /// Parses the supplied string as a WhatsApp intra-filename date or panics.
    fn parse_date_or_fail(date: &str) -> NaiveDate {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap_or_else(|e| panic!("Unable to parse `{}` as date: {}", date, e))
    }

    /// Gets the filename prefix for a path
    fn determine_filename_prefix(path: &Path) -> String {
        let filename =
            path.file_name().unwrap_or_else(|| panic!("Unable to determine filename of file: {}", path.display()));
        let filename = filename.to_str().unwrap_or_else(|| panic!("Filename is invalid UTF8: {:?}", filename));
        let prefix = filename.split('.').next().unwrap_or(filename).to_string();
        prefix
    }

    /// Removes old files from the `Backups` folder.
    ///
    /// This should correctly handle the case where the file extension changes
    /// since only the most recent file for a given prefix is kept. It won't
    /// handle the case where WhatsApp removes or changes the name
    /// (excluding file extension) of a backup file.
    pub fn clean_old_backups(&mut self) -> Result<(), Error> {
        // Get top-level files in `Backup`
        let backup_files_and_info: Vec<(PathBuf, FileInfo)> = self
            .entries
            .iter()
            .map(|(path, info)| (path.clone(), info.clone()))
            .filter(|(path, _)| {
                path.starts_with("Backups")
                    && path.components().count() == 2
                    && !path.file_name().and_then(|f| f.to_str()).map_or(true, |f| f.starts_with('.'))
            })
            .collect();
        // For each file prefix, determine the latest modified time.
        let mut latest: HashMap<String, FileTime> = HashMap::new();
        for (path, info) in &backup_files_and_info {
            let prefix = Self::determine_filename_prefix(path);
            let modification_time = info.get_modification_time();
            latest.entry(prefix).and_modify(|m| *m = std::cmp::max(*m, modification_time)).or_insert(modification_time);
        }
        // Delete all older files for each prefix
        for (path, info) in &backup_files_and_info {
            let modification_time = info.get_modification_time();
            let prefix = Self::determine_filename_prefix(path);
            let latest_modification_time =
                latest.get(&prefix).expect("Could not find latest modification time for prefix");
            if modification_time < *latest_modification_time {
                self.remove_file(path)?;
            }
        }
        Ok(())
    }

    fn clean_previous_dbs(&mut self, keep: usize) -> Result<(), Error> {
        let db_regex = Regex::new(r"msgstore(?P<incremental>-increment-\d+)?-(<?P<date>\d{4}-\d{2}-\d{2})\.")
            .expect("Invalid database name regex");
        let path_dates: Vec<(PathBuf, NaiveDate)> = self
            .entries
            .keys()
            .filter(|p| p.starts_with("Databases"))
            .filter_map(|p| {
                db_regex.captures(&p.to_string_lossy()).map(|captures| {
                    (
                        p.clone(),
                        Self::parse_date_or_fail(captures.name("date").expect("Date regex capture missing").as_str()),
                    )
                })
            })
            .collect();
        let unique_dates: BTreeSet<_> = path_dates.iter().map(|(_, date)| std::cmp::Reverse(*date)).collect();
        if unique_dates.len() <= keep {
            return Ok(());
        }
        let oldest_date_to_keep = unique_dates.into_iter().map(|d| d.0).take(keep).last().unwrap_or(NaiveDate::MAX);
        let to_delete: Vec<_> =
            path_dates.iter().filter(|(_, date)| *date < oldest_date_to_keep).map(|(path, _)| path).collect();
        for db in to_delete {
            self.remove_file(db)?;
        }
        Ok(())
    }

    fn clean_current_db(&mut self) -> Result<(), Error> {
        // Matches the current database backup, including incrementals.
        let db_regex = Regex::new(r"msgstore(?P<incremental>-increment-\d+)?\.db\.(?P<extension>.*)")
            .expect("Invalid database name regex");

        // Collect info for all database files
        let db_infos: Vec<(PathBuf, DbInfo)> = self
            .entries
            .iter()
            .map(|p| (p.0.clone(), p.1.clone()))
            .filter_map(|(path, file_info)| {
                if !path.starts_with("Databases") {
                    return None;
                }
                let capture =
                    path.file_name().and_then(|name| name.to_str()).and_then(|filename| db_regex.captures(filename));
                capture.map(|capture| {
                    (
                        path.clone(),
                        DbInfo {
                            last_modified: file_info.get_modification_time(),
                            file_extension: capture
                                .name("extension")
                                .expect("file extension unexpectedly missing")
                                .as_str()
                                .to_string(),
                            is_incremental: capture.name("incremental").is_some(),
                        },
                    )
                })
            })
            .collect();

        // Determine the most recent full backup (there might be multiple DBs with
        // different file extensions)
        let latest_db_info = db_infos
            .iter()
            .map(|(_, info)| info)
            .filter(|info| !info.is_incremental)
            .max_by_key(|i| i.last_modified)
            .expect("Unable to find current database");
        let file_extension = latest_db_info.file_extension.clone();
        let last_modified = latest_db_info.last_modified;

        // Delete any DBs not in the currently used format, or incremental backups that
        // are older than the last full backup.
        for (path, info) in db_infos {
            let incorrect_db_type = info.file_extension != file_extension;
            let outdated_increment = info.is_incremental && info.last_modified < last_modified;
            if incorrect_db_type || outdated_increment {
                self.remove_file(&path)?;
            }
        }
        Ok(())
    }

    /// Removes all but the last `keep` full WhatsApp backup databases
    pub fn clean_old_dbs(&mut self, keep: usize) -> Result<(), Error> {
        self.clean_previous_dbs(keep)?;
        self.clean_current_db()?;
        Ok(())
    }

    /// Mirrors the specified files from the supplied index into this one
    pub fn mirror_specified<I: IntoIterator<Item = impl AsRef<Path>>>(
        &mut self, source_index: &FileIndex, files: I,
    ) -> Result<(), Error> {
        let files: HashSet<PathBuf> = files.into_iter().map(|p| p.as_ref().to_path_buf()).collect();
        let source: HashMap<PathBuf, FileInfo> = source_index
            .entries
            .iter()
            .filter(|(p, _)| files.contains(p.as_path()))
            .map(|p| (p.0.clone(), p.1.clone()))
            .collect();
        if files.len() != source.len() {
            return Err(Error::IndexEntryMissing);
        }
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
        Ok(())
    }

    /// Mirrors all files from the supplied index into this one
    pub fn mirror_all(&mut self, source_index: &FileIndex) -> Result<(), Error> {
        self.mirror_specified(source_index, source_index.entries.keys())
    }

    /// The total size of all files in the index in bytes
    pub fn size_bytes(&self) -> u64 { self.entries.values().map(FileInfo::get_size).sum() }

    /// Returns true if this is a media file
    fn is_media_file(path: &Path, _file_info: &FileInfo) -> bool {
        path.starts_with("Media") && path.file_name().map_or(true, |e| e != ".nomedia")
    }

    /// Iterator over all media files
    fn media_files(&self) -> impl Iterator<Item = (&Path, &FileInfo)> {
        self.entries.iter().filter(|(p, fi)| Self::is_media_file(p, fi)).map(|(p, fi)| (p.as_path(), fi))
    }

    /// Iterator over non-media files
    fn non_media_files(&self) -> impl Iterator<Item = (&Path, &FileInfo)> {
        self.entries.iter().filter(|(p, fi)| !Self::is_media_file(p, fi)).map(|(p, fi)| (p.as_path(), fi))
    }

    /// Size of all media files in the index
    pub fn media_size_bytes(&self) -> u64 { self.media_files().map(|(_p, fi)| fi.get_size()).sum() }

    /// Size of all non-media files in the index
    pub fn non_media_size_bytes(&self) -> u64 { self.non_media_files().map(|(_p, fi)| fi.get_size()).sum() }

    /// Returns which files should be added and removed to satisfy the query
    pub fn get_delete_retain_candidates(&self, query: &FileQuery) -> (Vec<PathBuf>, Vec<PathBuf>) {
        // Construct list of media files
        let mut media_entries: Vec<(PathBuf, FileInfo)> =
            self.media_files().map(|(k, v)| (k.to_path_buf(), v.clone())).collect();
        let calculate_priority = |file_info: &FileInfo| -> (i32, f64) {
            // We assign a higher class to the files the user specifically requested we keep
            let class = i32::from(query.priority.matches(file_info));
            let value = query.order.evaluate(file_info);
            (class, value)
        };
        media_entries.sort_unstable_by(|(_, a), (_, b)| {
            calculate_priority(a).partial_cmp(&calculate_priority(b)).expect("Unable to compute ordering")
        });
        let (to_delete, to_retain) = match query.data_limit {
            DataLimit::Infinite => (Vec::new(), media_entries),
            DataLimit::Bytes(limit) => {
                let mut total: u64 = self.media_size_bytes();
                let mut count = 0;
                for (idx, (_, entry)) in media_entries.iter().enumerate() {
                    count = idx;
                    if total <= limit {
                        break;
                    }
                    total = total.saturating_sub(entry.get_size());
                }
                let to_retain = media_entries.split_off(count);
                let to_delete = media_entries;
                (to_delete, to_retain)
            }
        };
        (to_delete.into_iter().map(|(p, _)| p).collect(), to_retain.into_iter().map(|(p, _)| p).collect())
    }

    /// Returns all paths present in the index
    pub fn get_all_paths(&self) -> Vec<PathBuf> { self.entries.keys().cloned().collect() }

    /// Returns only the files which should be removed to satisfy the query
    pub fn get_delete_candidates(&self, query: &FileQuery) -> Vec<PathBuf> {
        self.get_delete_retain_candidates(query).0
    }

    /// Returns only the files which should be kept to satisfy the query
    pub fn get_retain_candidates(&self, query: &FileQuery) -> Vec<PathBuf> {
        self.get_delete_retain_candidates(query).1
    }

    /// Returns all files in `list` which are present in the index
    pub fn filter_existing(&self, list: &[PathBuf]) -> Vec<PathBuf> {
        list.iter().filter(|p| self.entries.contains_key(p.as_path())).cloned().collect()
    }

    /// Returns all files in `list` which are not in the index
    pub fn filter_missing(&self, list: &[PathBuf]) -> Vec<PathBuf> {
        list.iter().filter(|p| !self.entries.contains_key(p.as_path())).cloned().collect()
    }

    /// Removes files from the index and filesystem
    pub fn remove_files<I: IntoIterator<Item = impl AsRef<Path>>>(&mut self, files: I) -> Result<(), Error> {
        for file in files {
            self.remove_file(file.as_ref())?;
        }
        Ok(())
    }
}
