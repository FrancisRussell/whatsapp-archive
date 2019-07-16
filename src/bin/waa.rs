extern crate clap;
extern crate waa;
use clap::{App,Arg};
use waa::{FileIndex, FileQuery, FileOrder, IndexType, DataLimit};

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
             .help("Limit on size of WhatsApp folder in MB")
             .required(false)
             .value_name("size_limit"));

    let matches = app.get_matches();
    let wa_folder = matches.value_of("WHATSAPP_STORAGE").unwrap();
    let archive_folder = matches.value_of("ARCHIVE").unwrap();
    println!("Archiving from {} to {}", wa_folder, archive_folder);

    let mut wa_index = FileIndex::new(IndexType::Original, wa_folder).expect("Unable to index WhatsApp folder");
    let mut archive_index = FileIndex::new(IndexType::Archive, archive_folder).expect("Unable to index archive folder");
    archive_index.mirror_from(&wa_index).expect("Unable to mirror WhatsApp folder");

    let limit = matches.value_of("LIMIT")
        .map(|v| v.parse::<u64>().expect("Unable to parse LIMIT"))
        .map(|v| DataLimit::from_bytes(v * 1024 * 1024)).unwrap_or(DataLimit::Infinite);
    let mut query = FileQuery::new();
    query.set_order(FileOrder::LargestOldest);
    query.set_limit(limit);
    let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("WhatsApp folder size is {:.2} MB", wa_folder_size_mb);
    println!("Archive size is {:.2} MB", archive_size_mb);
    let deletion_candidates = archive_index.get_deletion_candidates(&query);
    let deletion_candidates = wa_index.filter_existing(&deletion_candidates);
    println!("Deleting {} files from WhatsApp folder...", deletion_candidates.len());
    wa_index.delete_files_from_infos(&deletion_candidates).expect("Unable to trim files from WhatsApp folder");
}
