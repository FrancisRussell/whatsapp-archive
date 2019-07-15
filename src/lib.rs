#[macro_use] extern crate derive_error;
#[macro_use] extern crate log;
extern crate filetime;
extern crate regex;
mod file_index;
pub use self::file_index::{FileIndex, IndexType};
