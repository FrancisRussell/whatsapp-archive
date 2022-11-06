extern crate bytefmt;
extern crate clap;
extern crate waa;
use std::str::FromStr;

use clap::{App, Arg};
use waa::{ActionType, DataLimit, FileFilter, FileIndex, FileQuery, FileScore, IndexType};

fn main() {
    match main_internal() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationMode {
    Backup,
    Trim,
    Sync,
}

#[derive(Clone, Copy, Debug)]
pub enum ParseOperationModeError {
    UnknownOperationMode,
}

impl FromStr for OperationMode {
    type Err = ParseOperationModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_string();
        match s.as_ref() {
            "backup" => Ok(OperationMode::Backup),
            "trim" => Ok(OperationMode::Trim),
            "sync" => Ok(OperationMode::Sync),
            _ => Err(ParseOperationModeError::UnknownOperationMode),
        }
    }
}

fn main_internal() -> Result<(), String> {
    let app = App::new("WhatsApp Archiver")
        .author("Francis Russell")
        .version("0.1.0")
        .arg(
            Arg::with_name("WHATSAPP_STORAGE")
                .short('w')
                .help("Location of WhatsApp folder")
                .required(true)
                .value_name("whatsapp_folder"),
        )
        .arg(
            Arg::with_name("ARCHIVE")
                .short('a')
                .help("Location of archive folder")
                .required(true)
                .value_name("archive_folder"),
        )
        .arg(
            Arg::with_name("LIMIT")
                .short('l')
                .help("Limit on size of WhatsApp folder with suffix e.g. 512MiB")
                .required(false)
                .value_name("size_limit"),
        )
        .arg(
            Arg::with_name("DRY_RUN")
                .short('n')
                .long("dry-run")
                .help("Print actions without modifying filesystem")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("MIN_AGE_DAYS")
                .required(false)
                .long("min-age")
                .short('m')
                .help("Minimum age of any deleted media files in days")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("ORDER")
                .required(false)
                .short('o')
                .long("order")
                .help(
                    "Which files to delete first (ONLY media):\n\
                  \toldest - preserve the most history\n\
                  \tlargest - remove the fewest files\n\
                  \tlargest_oldest - tries to balance largest and oldest\n",
                )
                .default_value("largest_oldest"),
        )
        .arg(
            Arg::with_name("MODE")
                .short('M')
                .long("mode")
                .help(
                    "Mode to run in:\n\
                \tbackup - updates archive from WhatsApp folder\n\
                \ttrim - same as backup, but also removes files from WhatsApp folder\n\
                \tsync - same as trim, but also restores files to WhatsApp folder (ONLY media)\n",
                )
                .default_value("backup"),
        )
        .arg(
            Arg::with_name("NUM_KEPT_DBS")
                .short('k')
                .long("kept-dbs")
                .help("Number of message database backups to retain in archive")
                .default_value("10"),
        );

    let matches = app.get_matches();
    let wa_folder = matches.value_of("WHATSAPP_STORAGE").unwrap();
    let archive_folder = matches.value_of("ARCHIVE").unwrap();

    if matches.value_of("LIMIT").and_then(|v| v.parse::<usize>().ok()).is_some() {
        panic!("LIMIT must include a suffix e.g. 12MiB");
    }

    let limit = matches
        .value_of("LIMIT")
        .map(|v| bytefmt::parse(v).expect("Unable to parse LIMIT"))
        .map(DataLimit::from_bytes)
        .unwrap_or(DataLimit::Infinite);

    let min_age = matches
        .value_of("MIN_AGE_DAYS")
        .map(|v| v.parse::<u32>().expect("Unable to parse MIN_AGE_DAYS"))
        .map(FileFilter::MinAgeDays)
        .unwrap_or(FileFilter::All);

    let mode =
        matches.value_of("MODE").map(|v| v.parse::<OperationMode>().expect("Unable to parse operation mode")).unwrap();

    let order = matches.value_of("ORDER").map(|v| v.parse::<FileScore>().expect("Unable to parse file order")).unwrap();

    let num_dbs_to_keep = matches
        .value_of("NUM_KEPT_DBS")
        .map(|v| v.parse::<usize>().expect("Unable to parse number of kept databases"))
        .unwrap();

    let action_type = if matches.is_present("DRY_RUN") {
        println!("Running in dry-run mode. No files will be changed.");
        ActionType::Dry
    } else {
        ActionType::Real
    };

    let mut wa_index = match FileIndex::new(IndexType::Original, wa_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index WhatsApp folder: {}", e)),
    };

    let mut archive_index = match FileIndex::new(IndexType::Archive, archive_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index archive folder: {}", e)),
    };

    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Mirroring new files from {} to {}...", wa_folder, archive_folder);
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
        query.set_filter(min_age);
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
