#![warn(clippy::pedantic)]
#![allow(clippy::uninlined_format_args, clippy::doc_markdown)]

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use thiserror::Error;
use waa::{ActionType, DataLimit, Error, FileIndex, FilePredicate, FileQuery, FileScore, IndexType};

fn main() {
    if let Err(e) = main_internal() {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OperationMode {
    /// updates archive from WhatsApp folder
    #[clap(name = "backup")]
    Backup,

    /// same as backup, but also removes files from WhatsApp folder
    #[clap(name = "trim")]
    Trim,

    /// same as trim, but also restores files to WhatsApp folder (ONLY media)
    #[clap(name = "sync")]
    Sync,
}

#[derive(Clone, Copy, Debug)]
pub enum ParseOperationModeError {
    UnknownOperationMode,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum FileOrdering {
    /// keep the most contiguous history
    #[clap(name = "newer")]
    Newer,

    /// keep the most files
    #[clap(name = "smaller")]
    Smaller,

    /// tries to balance between newer and smaller
    #[clap(name = "smaller_newer")]
    SmallerNewer,
}

impl From<FileOrdering> for FileScore {
    fn from(o: FileOrdering) -> FileScore {
        match o {
            FileOrdering::Newer => FileScore::Newer,
            FileOrdering::Smaller => FileScore::Smaller,
            FileOrdering::SmallerNewer => FileScore::SmallerNewer,
        }
    }
}

// Using `bytefmt::parse` directly angers `clap`
fn parse_byte_count(s: &str) -> Result<u64, &'static str> { bytefmt::parse(s) }

#[derive(Debug, Parser)]
#[clap(author, version, about = "WhatsApp Archiver")]
struct Cli {
    #[clap(short = 'w')]
    /// Location of WhatsApp folder
    whatsapp_folder: PathBuf,

    #[clap(short = 'a')]
    /// Location of archive folder
    archive_folder: PathBuf,

    #[clap(short='l', value_parser = parse_byte_count)]
    /// Limit on size of WhatsApp folder with suffix e.g. 512MiB
    size_limit: Option<u64>,

    #[clap(short = 'n', long = "dry-run", action)]
    /// Print actions without modifying filesystem
    dry_run: bool,

    #[clap(long = "keep-newer-than", value_parser = humantime::parse_duration)]
    /// Prioritise keeping files newer than this duration e.g. 7d
    keep_newer_than: Option<std::time::Duration>,

    #[clap(value_enum, short='o', long="order", default_value_t = FileOrdering::SmallerNewer)]
    /// Which files to try to keep on phone (ONLY media)
    order: FileOrdering,

    #[clap(value_enum, short = 'M', long = "mode", default_value_t = OperationMode::Backup)]
    /// Mode of operation
    mode: OperationMode,

    #[clap(short = 'k', long = "kept-dbs", default_value_t = 10)]
    /// Number of message database backups to retain in archive
    num_kept_dbs: usize,
}

#[derive(Debug, Error)]
enum AppError {
    /// Error building a file index
    #[error("Unable to build index of {0}: {1}")]
    BuildIndex(PathBuf, Error),

    /// Failure during file mirroring to backup
    #[error("Unable to mirror files to archive: {0}")]
    MirrorToArchive(Error),

    /// Failure while trimming files from WhatsApp folder
    #[error("Unable to trim files from WhatsApp folder: {0}")]
    TrimWhatsApp(Error),

    /// Failure while cleaning up archive
    #[error("Unable to clean unnecessary files from archive folder: {0}")]
    TidyArchive(Error),

    /// Failure while restoring files to WhatsApp folder
    #[error("Unable to restore files to WhatsApp folder: {0}")]
    RestoreToWhatsApp(Error),
}

fn main_internal() -> Result<(), AppError> {
    let cli = Cli::parse();
    let wa_folder = cli.whatsapp_folder;
    let archive_folder = cli.archive_folder;

    let limit = cli.size_limit.map_or(DataLimit::Infinite, DataLimit::from_bytes);

    let priority = cli
        .keep_newer_than
        .map(|d| chrono::Duration::from_std(d).expect("Duration too large"))
        .map_or(FilePredicate::Constant(false), FilePredicate::AgeLessThan);

    let mode = cli.mode;
    let order: FileScore = cli.order.into();
    let num_dbs_to_keep = cli.num_kept_dbs;

    let action_type = if cli.dry_run {
        println!("Running in dry-run mode. No files will be changed.");
        ActionType::Dry
    } else {
        ActionType::Real
    };

    let mut wa_index = FileIndex::new(IndexType::Original, &wa_folder, action_type)
        .map_err(|e| AppError::BuildIndex(wa_folder.clone(), e))?;

    let mut archive_index = FileIndex::new(IndexType::Archive, &archive_folder, action_type)
        .map_err(|e| AppError::BuildIndex(wa_folder.clone(), e))?;

    let archive_size = archive_index.size_bytes();
    println!("Mirroring new files from {} to {}...", wa_folder.display(), archive_folder.display());
    println!("Archive size is currently {}", bytefmt::format(archive_size));

    archive_index.mirror_all(&wa_index).map_err(AppError::MirrorToArchive)?;
    archive_index.clean_old_backups().map_err(AppError::TidyArchive)?;
    archive_index.clean_old_dbs(num_dbs_to_keep).map_err(AppError::TidyArchive)?;

    let archive_size = archive_index.size_bytes();
    println!("Archive size is now {}", bytefmt::format(archive_size));

    if mode == OperationMode::Trim || mode == OperationMode::Sync {
        println!("\nTrimming files from WhatsApp folder...");
        let wa_folder_size = wa_index.size_bytes();
        println!("WhatsApp folder size is currently {}", bytefmt::format(wa_folder_size));

        let mut query = FileQuery::default();
        query.set_order(order);
        query.set_priority(priority);
        let limit = limit.map(|bytes| {
            // Reduce limit to account for non-media files in WhatsApp folder
            let non_media_bytes = wa_index.non_media_size_bytes();
            bytes.saturating_sub(non_media_bytes)
        });
        query.set_limit(limit);

        let (delete_candidates, retain_candidates) = {
            let deletion_source = match mode {
                OperationMode::Trim => &wa_index,
                OperationMode::Sync => &archive_index,
                OperationMode::Backup => panic!("Delete/retain should never be hit in backup mode"),
            };
            deletion_source.get_delete_retain_candidates(&query)
        };
        let delete_candidates = wa_index.filter_existing(&delete_candidates);
        println!("Deleting {} files from WhatsApp folder...", delete_candidates.len());

        wa_index.remove_files(&delete_candidates).map_err(AppError::TrimWhatsApp)?;
        if !delete_candidates.is_empty() {
            let wa_folder_size = wa_index.size_bytes();
            println!("WhatsApp folder size is now {}", bytefmt::format(wa_folder_size));
        }

        if mode == OperationMode::Sync {
            let restore_candidates = wa_index.filter_missing(&retain_candidates);
            println!("\nRestoring {} files to WhatsApp folder...", restore_candidates.len());
            wa_index.mirror_specified(&archive_index, &restore_candidates).map_err(AppError::RestoreToWhatsApp)?;

            if !restore_candidates.is_empty() {
                let wa_folder_size = wa_index.size_bytes();
                println!("WhatsApp folder size is now {}", bytefmt::format(wa_folder_size));
            }
        }
    }
    println!("Done.");
    Ok(())
}
