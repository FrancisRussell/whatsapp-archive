use std::io;
use std::path::{Path, PathBuf};

use err_derive::Error;

#[derive(Debug, Error)]
pub enum FileIndexError {
    #[error(display = "An IO error occurred involving {:?}: {}", _1, _0)]
    Io(io::Error, PathBuf),

    #[error(display = "An IO error occurred while copying: {}\nSource: {:?}\nTarget:{:?}", _0, _1, _2)]
    Cp(io::Error, PathBuf, PathBuf),

    #[error(display = "The supplied folder was not a WhatsApp folder: {:?}", _0)]
    NotWhatsAppFolder(PathBuf),

    #[error(display = "The supplied folder was not an archive folder but not empty: {:?}", _0)]
    NewArchiveFolderNotEmpty(PathBuf),

    #[error(
        display = "After a copy operation, the metadata of the two files did not match:\nSource: {:?}\nTarget: {:?}",
        _0,
        _1
    )]
    FileMismatch(PathBuf, PathBuf),

    #[error(display = "A file was unexpectedly missing: {:?}", _0)]
    FileMissing(PathBuf),

    #[error(display = "An entry was unexpectedly missing from the file index (probably a bug)")]
    IndexEntryMissing,
}

impl<P: AsRef<Path>> From<(io::Error, P)> for FileIndexError {
    fn from(err: (io::Error, P)) -> Self { FileIndexError::Io(err.0, err.1.as_ref().to_owned()) }
}
