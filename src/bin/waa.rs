extern crate clap;
extern crate waa;
extern crate bytefmt;
use clap::{App,Arg};
use waa::{FileIndex, FileQuery, FileScore, IndexType, DataLimit, ActionType, FileFilter};

fn main() {
    match main_internal() {
        Ok(_) => {},
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        },
    };
}

fn main_internal() -> Result<(), String> {
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
            .takes_value(false))
        .arg(Arg::with_name("MIN_AGE_DAYS")
            .required(false)
            .long("min-age")
            .short("m")
            .help("Minimum age of any deleted files in days")
            .takes_value(true))
        .arg(Arg::with_name("ORDER")
            .required(false)
            .short("o")
            .long("order")
            .help("Which files to delete first:\n\
                  \toldest - preserve the most history\n\
                  \tlargest - remove the fewest files\n\
                  \tlargest_oldest - tries to balance largest and oldest\n")
            .default_value("largest_oldest"));

    let matches = app.get_matches();
    let wa_folder = matches.value_of("WHATSAPP_STORAGE").unwrap();
    let archive_folder = matches.value_of("ARCHIVE").unwrap();
    if let Some(_) = matches.value_of("LIMIT").and_then(|v| v.parse::<usize>().ok()) {
        panic!("LIMIT must include a suffix e.g. 12MiB");
    }
    let limit = matches.value_of("LIMIT")
        .map(|v| bytefmt::parse(v).expect("Unable to parse LIMIT"))
        .map(|v| DataLimit::from_bytes(v)).unwrap_or(DataLimit::Infinite);
    let min_age = matches.value_of("MIN_AGE_DAYS")
        .map(|v| v.parse::<u32>().expect("Unable to parse MIN_AGE_DAYS"))
        .map(|v| FileFilter::MinAgeDays(v))
        .unwrap_or(FileFilter::All);
    let action_type = if matches.is_present("DRY_RUN") {
        println!("Running in dry-run mode. No files will be changed.");
        ActionType::Dry
    } else {
        ActionType::Real
    };
    let order = matches.value_of("ORDER")
        .map(|v| v.parse::<FileScore>().expect("Unable to parse file order")).unwrap();
    let mut wa_index = match FileIndex::new(IndexType::Original, wa_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index WhatsApp folder: {}", e)),
    };
    let mut archive_index = match FileIndex::new(IndexType::Archive, archive_folder, action_type) {
        Ok(i) => i,
        Err(e) => return Err(format!("Unable to index archive folder: {}", e)),
    };
    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Archive size is currently {:.2} MB", archive_size_mb);
    println!("Mirroring new files from {} to {}...", wa_folder, archive_folder);
    match archive_index.mirror_from(&wa_index) {
        Ok(()) => {},
        Err(e) => return Err(format!("Error while mirroring WhatsApp folder: {}", e)),
    };
    println!("Mirroring complete.");
    let archive_size_mb = (archive_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("Archive size is now {:.2} MB", archive_size_mb);

    let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
    println!("WhatsApp folder size is currently {:.2} MB", wa_folder_size_mb);

    let mut query = FileQuery::new();
    query.set_order(order);
    query.set_limit(limit);
    query.set_filter(min_age);
    let deletion_candidates = archive_index.get_deletion_candidates(&query);
    let deletion_candidates = deletion_candidates.iter().map(|(path, _)| path).cloned().collect();
    let deletion_candidates = wa_index.filter_existing(&deletion_candidates);
    println!("Deleting {} files from WhatsApp folder...", deletion_candidates.len());
    match wa_index.remove_files(&deletion_candidates) {
        Ok(()) => {},
        Err(e) => return Err(format!("Error while trimming files from WhatsApp folder: {}", e)),
    };
    if !deletion_candidates.is_empty() {
        let wa_folder_size_mb = (wa_index.get_size_bytes() as f64) / (1024.0 * 1024.0);
        println!("WhatsApp folder size is now {:.2} MB", wa_folder_size_mb);
    }
    Ok(())
}

