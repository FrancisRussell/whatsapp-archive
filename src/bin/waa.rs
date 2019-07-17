extern crate clap;
extern crate waa;
extern crate bytefmt;
use clap::{App,Arg};
use waa::{FileIndex, FileQuery, FileOrder, IndexType, DataLimit, ActionType};

fn main() {
    let app = App::new("WhatsApp Archiver")
        .author("Francis Russell")
        .arg(Arg::with_name("WHATSAPP_STORAGE")
            .short("w")
            .help("Location of WhatsApp folder")
            .required(true)
            .value_name("whatsapp_folder"))
        .arg(Arg::with_name("ARCHIVE")
            .short("a")
            .help("Location of archive folder")
            .required(true)
            .value_name("archive_folder"))
        .arg(Arg::with_name("LIMIT")
             .short("l")
             .help("Limit on size of WhatsApp folder with suffix e.g. 512MiB")
             .required(false)
             .value_name("size_limit"))
        .arg(Arg::with_name("DRY_RUN")
            .short("n")
            .long("dry-run")
            .help("Print actions without modifying filesystem")
            .required(false)
            .takes_value(false));

    let matches = app.get_matches();
    let wa_folder = matches.value_of("WHATSAPP_STORAGE").unwrap();
    let archive_folder = matches.value_of("ARCHIVE").unwrap();
    if let Some(_) = matches.value_of("LIMIT").and_then(|v| v.parse::<usize>().ok()) {
        panic!("LIMIT must include a suffix e.g. 12MiB");
    }
    let limit = matches.value_of("LIMIT")
        .map(|v| bytefmt::parse(v).expect("Unable to parse LIMIT"))
        .map(|v| DataLimit::from_bytes(v)).unwrap_or(DataLimit::Infinite);

    let action_type = if matches.is_present("DRY_RUN") {
        println!("Running in dry-run mode. No files will be changed.");
        ActionType::Dry
    } else {
        ActionType::Real
    };
    let mut wa_index = FileIndex::new(IndexType::Original, wa_folder, action_type).expect("Unable to index WhatsApp folder");
    let mut archive_index = FileIndex::new(IndexType::Archive, archive_folder, action_type).expect("Unable to index archive folder");
    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Archive size is currently {:.2} MB", archive_size_mb);
    println!("Mirroring new files from {} to {}...", wa_folder, archive_folder);
    archive_index.mirror_from(&wa_index).expect("Unable to mirror WhatsApp folder");
    println!("Mirroring complete.");
    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Archive size is now {:.2} MB", archive_size_mb);

    let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("WhatsApp folder size is currently {:.2} MB", wa_folder_size_mb);

    let mut query = FileQuery::new();
    query.set_order(FileOrder::LargestOldest);
    query.set_limit(limit);
    let deletion_candidates = archive_index.get_deletion_candidates(&query);
    let deletion_candidates = deletion_candidates.iter().map(|(path, _)| path).cloned().collect();
    let deletion_candidates = wa_index.filter_existing(&deletion_candidates);
    println!("Deleting {} files from WhatsApp folder...", deletion_candidates.len());
    wa_index.remove_files(&deletion_candidates).expect("Unable to trim files from WhatsApp folder");
    if !deletion_candidates.is_empty() {
        let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
        println!("WhatsApp folder size is now {:.2} MB", wa_folder_size_mb);
    }

}

