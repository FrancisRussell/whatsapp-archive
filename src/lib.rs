#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

mod error;
mod file_index;
mod file_info;
mod filter;

pub use error::Error;
pub use file_index::{ActionType, FileIndex, IndexType};
pub use file_info::FileInfo;
pub use filter::{DataLimit, FilePredicate, FileQuery, FileScore};
