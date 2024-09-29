use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// An IO error involving a path
    #[error("An IO error occurred involving {1}: {0}")]
    Io(io::Error, PathBuf),

    /// An IO error encountered during a file copy
    #[error("An IO error occurred while copying: {0}\nSource: {1}\nTarget:{2}")]
    Cp(io::Error, PathBuf, PathBuf),

    /// An IO error encountered during a file rename
    #[error("An IO error occurred while renaming: {0}\nSource: {1}\nTarget:{2}")]
    Mv(io::Error, PathBuf, PathBuf),

    /// The supplied folder was not a WhatsApp data folder
    #[error("The supplied folder was not a WhatsApp folder: {0}")]
    NotWhatsAppFolder(PathBuf),

    /// The supplied folder was neither an exising WhatsApp backup folder nor
    /// empty
    #[error("The supplied folder was not an archive folder but not empty: {0}")]
    NewArchiveFolderNotEmpty(PathBuf),

    /// Failed to copy file metadata
    #[error("After a copy operation, the metadata of the two files did not match:\nSource: {0}\nTarget: {1}")]
    FileMismatch(PathBuf, PathBuf),

    /// File not found
    #[error("A file was unexpectedly missing: {0}")]
    FileMissing(PathBuf),

    /// An entry in the file index was unexpectedly missing
    #[error("An entry was unexpectedly missing from the file index (probably a bug)")]
    IndexEntryMissing,
}

impl<P: AsRef<Path>> From<(io::Error, P)> for Error {
    fn from(err: (io::Error, P)) -> Self { Error::Io(err.0, err.1.as_ref().to_owned()) }
}
