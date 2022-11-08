### WhatsApp Archiver

Copies files from an Android WhatsApp folder to an archive folder and
trims media files from the source.

`waa` is written in Rust.

Usage:

``` 
$ waa -a <archive_folder> -w <whatsapp_folder>
  [-l <size_limit>] [--dry-run] [--keep-newer-than DURATION]
  [-o|--order newer|smaller|smaller_newer] [-M|--mode backup|trim|sync]
  [-k|--num-kept-dbs NUM_KEPT_DBS]
```

e.g.

``` 
$ waa -a ${HOME}/whatsapp_backup 
  -w /mnt/phone/Android/media/com.whatsapp/WhatsApp/
  -l 512MiB -M newer --keep-newer-than 14d
```

In all modes, all files not present in `archive_folder` will be copied from
`whatsapp_folder` preserving file modification times. This is the only operation
that occurs in `backup` mode.

In `trim` mode, files will be removed from the WhatsApp folder to reduce its size
to be under the specified limit.

In `sync` mode, files may be both removed and added from the WhatsApp folder in order
to satisfy the `--order` and `--keep-newer-than` preferences while keeping the folder
under the specified size limit.

The order `newer` weights newer files over older ones and therefore preserves
the most contiguous media history. The order `smaller` weights smaller files
over larger ones and therefore will preserve smaller files like pictures before
retaining videos. `smaller_newer` attempts to produce a balance in which
smaller files are preserved but files also become less important with age.
