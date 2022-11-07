use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use waa::{ActionType, DataLimit, FileIndex, FilePredicate, FileQuery, FileScore, IndexType};

fn main() {
    match main_internal() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
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

    #[clap(long = "keep-newer-than", value_parser = parse_duration::parse)]
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

fn main_internal() -> Result<(), String> {
    let cli = Cli::parse();
    let wa_folder = cli.whatsapp_folder;
    let archive_folder = cli.archive_folder;

    let limit = cli.size_limit.map(DataLimit::from_bytes).unwrap_or(DataLimit::Infinite);

    let priority = cli
        .keep_newer_than
        .map(|d| chrono::Duration::from_std(d).expect("Duration too large"))
        .map(FilePredicate::AgeLessThan)
        .unwrap_or(FilePredicate::Constant(false));

    let mode = cli.mode;
    let order: FileScore = cli.order.into();
    let num_dbs_to_keep = cli.num_kept_dbs;

    let action_type = if cli.dry_run {
        println!("Running in dry-run mode. No files will be changed.");
        ActionType::Dry
    } else {
        ActionType::Real
    };

    let mut wa_index = match FileIndex::new(IndexType::Original, &wa_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index WhatsApp folder: {}", e)),
    };

    let mut archive_index = match FileIndex::new(IndexType::Archive, &archive_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index archive folder: {}", e)),
    };

    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Mirroring new files from {} to {}...", wa_folder.display(), archive_folder.display());
    println!("Archive size is currently {:.2} MB", archive_size_mb);

    if let Err(e) = archive_index.mirror_all(&wa_index) {
        return Err(format!("Error while mirroring WhatsApp folder: {}", e));
    }

    if let Err(e) = archive_index.clean_old_dbs(num_dbs_to_keep) {
        return Err(format!("Error while deleting old databases from archive folder: {}", e));
    }

    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Archive size is now {:.2} MB", archive_size_mb);

    if mode == OperationMode::Trim || mode == OperationMode::Sync {
        println!("\nTrimming files from WhatsApp folder...");
        let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
        println!("WhatsApp folder size is currently {:.2} MB", wa_folder_size_mb);

        let mut query = FileQuery::default();
        query.set_order(order);
        query.set_limit(limit);
        query.set_priority(priority);
        let (delete_candidates, retain_candidates) = {
            let deletion_source = match mode {
                OperationMode::Trim => &wa_index,
                OperationMode::Sync => &archive_index,
                _ => panic!("Unexpected mode of operation"),
            };
            deletion_source.get_delete_retain_candidates(&query)
        };
        let delete_candidates = wa_index.filter_existing(&delete_candidates);
        println!("Deleting {} files from WhatsApp folder...", delete_candidates.len());
        match wa_index.remove_files(&delete_candidates) {
            Ok(()) => {}
            Err(e) => return Err(format!("Error while trimming files from WhatsApp folder: {}", e)),
        };
        if !delete_candidates.is_empty() {
            let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
            println!("WhatsApp folder size is now {:.2} MB", wa_folder_size_mb);
        }

        if mode == OperationMode::Sync {
            let restore_candidates = wa_index.filter_missing(&retain_candidates);
            println!("\nRestoring {} files to WhatsApp folder...", restore_candidates.len());

            if let Err(e) = wa_index.mirror_specified(&archive_index, &restore_candidates) {
                return Err(format!("Error while restoring files to WhatsApp folder: {}", e));
            }

            if !restore_candidates.is_empty() {
                let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
                println!("WhatsApp folder size is now {:.2} MB", wa_folder_size_mb);
            }
        }
    }
    println!("Done.");
    Ok(())
}
