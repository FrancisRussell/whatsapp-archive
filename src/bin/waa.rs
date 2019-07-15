extern crate clap;
extern crate waa;
use clap::{App,Arg};
use waa::{FileIndex, IndexType};

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
            .value_name("archive_folder"));

    let matches = app.get_matches();
    let wa_folder = matches.value_of("WHATSAPP_STORAGE").unwrap();
    let archive_folder = matches.value_of("ARCHIVE").unwrap();
    println!("Archiving from {} to {}", wa_folder, archive_folder);

    let wa_index = FileIndex::new(IndexType::Original, wa_folder).expect("Unable to index WhatsApp folder");
    let mut archive_index = FileIndex::new(IndexType::Archive, archive_folder).expect("Unable to index archive folder");
    archive_index.mirror_from(&wa_index).expect("Unable to mirror WhatsApp folder");
}
