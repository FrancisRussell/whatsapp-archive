use std::collections::{hash_map, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use log::warn;
use regex::Regex;

use crate::{DataLimit, FileIndexError, FileInfo, FileQuery};

const TAG_NAME: &str = ".waa";

#[derive(Clone, Copy, Debug)]
pub enum IndexType {
    Original,
    Archive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionType {
    Real,
    Dry,
}

#[derive(Debug)]
pub struct FileIndex {
    _index_type: IndexType,
    action_type: ActionType,
    path: PathBuf,
    entries: HashMap<PathBuf, FileInfo>,
}

impl FileIndex {
    pub fn new<P: AsRef<Path>>(
        index_type: IndexType, path: P, action_type: ActionType,
    ) -> Result<FileIndex, FileIndexError> {
        let path = path.as_ref();
        let mut new = false;
        match index_type {
            IndexType::Original => {
                let db_path = path.join("Databases").join("msgstore.db.crypt14");
                let tag_path = path.join(TAG_NAME);
                if !db_path.exists() || tag_path.exists() {
                    return Err(FileIndexError::NotWhatsAppFolder(path.to_owned()));
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
                            return Err(FileIndexError::NewArchiveFolderNotEmpty(path.to_owned()));
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

    fn get_relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.path).expect("Unable to strip prefix").to_owned()
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

    fn import_file_maybe_metadata(
        &mut self, relative_path: &Path, source: &Path, info: Option<&FileInfo>,
    ) -> Result<(), FileIndexError> {
        let dest_path = self.path.join(relative_path);
        let mut do_copy = || {
            assert!(relative_path.is_relative());
            if self.action_type == ActionType::Real {
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| (e, parent))?;
                }
                std::fs::copy(source, &dest_path)
                    .map_err(|e| FileIndexError::Cp(e, source.to_owned(), dest_path.to_owned()))?;
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
                    let _ = std::fs::remove_file(&dest_path)
                        .map_err(|e| eprintln!("Additional error during delete of incompletely copied file: {:?}", e));
                }
                Err(e)
            }
        }
    }

    pub fn import_file(&mut self, relative_path: &Path, source: &Path) -> Result<(), FileIndexError> {
        self.import_file_maybe_metadata(relative_path, source, None)
    }

    pub fn import_file_with_metadata(
        &mut self, relative_path: &Path, source: &Path, info: &FileInfo,
    ) -> Result<(), FileIndexError> {
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

    pub fn clean_old_dbs(&mut self, keep: usize) -> Result<(), FileIndexError> {
        let db_regex = Regex::new(r"msgstore-\d{4}-\d{2}-\d{2}").unwrap();
        let mut paths: Vec<PathBuf> = self
            .entries
            .keys()
            .map(|rel_path| rel_path.to_owned())
            .filter(|p| p.starts_with("Databases"))
            .filter(|p| db_regex.is_match(p.to_string_lossy().as_ref()))
            .collect();
        if paths.len() <= keep {
            return Ok(());
        }
        paths.sort();
        let delete_count = paths.len() - keep;
        let to_delete = &paths[..delete_count];
        println!("Removing old databases from archive");
        for db in to_delete {
            self.remove_file(db)?;
        }
        Ok(())
    }

    pub fn mirror_specified<I: IntoIterator<Item = impl AsRef<Path>>>(
        &mut self, source_index: &FileIndex, files: I,
    ) -> Result<(), FileIndexError> {
        let files: HashSet<PathBuf> = files.into_iter().map(|p| p.as_ref().to_path_buf()).collect();
        let source: HashMap<PathBuf, FileInfo> = source_index
            .entries
            .iter()
            .filter(|(p, _)| files.contains(p.as_path()))
            .map(|p| (p.0.clone(), p.1.clone()))
            .collect();
        if files.len() != source.len() {
            return Err(FileIndexError::IndexEntryMissing);
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

    pub fn mirror_all(&mut self, source_index: &FileIndex) -> Result<(), FileIndexError> {
        self.mirror_specified(source_index, source_index.entries.keys())
    }

    pub fn get_size_bytes(&self) -> u64 { self.entries.values().map(|fi| fi.get_size()).sum() }

    pub fn get_delete_retain_candidates(&self, query: &FileQuery) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut media_entries: Vec<(PathBuf, FileInfo)> = self
            .entries
            .iter()
            .filter(|(p, _)| p.starts_with("Media"))
            .filter(|(p, _)| p.file_name().map(|e| e != ".nomedia").unwrap_or(true))
            .filter(|(_, i)| query.filter.matches(i))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        media_entries.sort_unstable_by(|(_, a), (_, b)| {
            query.order.evaluate(a).partial_cmp(&query.order.evaluate(b)).map(|v| v.reverse()).unwrap()
        });
        let (to_delete, to_retain) = match query.limit {
            DataLimit::Infinite => (Vec::new(), media_entries),
            DataLimit::Bytes(limit) => {
                let mut total: u64 = self.get_size_bytes();
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

    pub fn get_all_paths(&self) -> Vec<PathBuf> { self.entries.keys().cloned().collect() }

    pub fn get_delete_candidates(&self, query: &FileQuery) -> Vec<PathBuf> {
        self.get_delete_retain_candidates(query).0
    }

    pub fn get_retain_candidates(&self, query: &FileQuery) -> Vec<PathBuf> {
        self.get_delete_retain_candidates(query).1
    }

    pub fn filter_existing(&self, list: &[PathBuf]) -> Vec<PathBuf> {
        list.iter().filter(|p| self.entries.contains_key(p.as_path())).cloned().collect()
    }

    pub fn filter_missing(&self, list: &[PathBuf]) -> Vec<PathBuf> {
        list.iter().filter(|p| !self.entries.contains_key(p.as_path())).cloned().collect()
    }

    pub fn remove_files<I: IntoIterator<Item = impl AsRef<Path>>>(&mut self, files: I) -> Result<(), FileIndexError> {
        for file in files {
            self.remove_file(file.as_ref())?;
        }
        Ok(())
    }
}
