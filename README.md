### WhatsApp Archiver

Copies files from an Android WhatsApp folder to an archive folder and
trims media files from the source.

`waa` is written in Rust.

Usage:

``` 
$ cargo run --release -- -a <archive_folder> -w <whatsapp_folder>
  [-l <size_limit>] [--dry-run] [--min-age=DAYS]
```

e.g.

``` 
$ cargo run --release -- -a ${HOME}/whatsapp_backup 
  -w /mnt/phone/internal_storage/WhatsApp 
  -l 512MiB --min-age-days=14
```

All files not present in `archive_folder` will be copied from
`whatsapp_folder` preserving file modification times.

If a size limit is provided, media files from `whatsapp_folder` will be deleted
so that the size of the folder is under the specified limit (or as close as
possible).

Currently the files to be deleted are chosen using the weighting `file_size *
file_age`. This favours deleting videos over images, and older files over newer
ones.
