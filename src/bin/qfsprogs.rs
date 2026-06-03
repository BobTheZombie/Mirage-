use std::env;
use std::process::ExitCode;

use mirage::stdlib::qfs::{
    dump_superblock, fsck_image, mkfs_image, stat_image, QfsImageReport, QfsSuperblock,
    QFS_BOOK_PAGES, QFS_PAGE_SECTORS,
};

const DEFAULT_SECTORS: u64 = 1 + QFS_BOOK_PAGES as u64 * QFS_PAGE_SECTORS as u64;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("qfsprogs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    match command.as_str() {
        "mkfs" => {
            let image = args.next().ok_or_else(usage)?;
            let sectors = parse_optional_sectors(args)?;
            let report = mkfs_image(&image, sectors).map_err(|error| error.to_string())?;
            println!(
                "formatted {image}: sectors={} books={} free_sectors={}",
                report.sector_count, report.total_books, report.free_sectors
            );
        }
        "fsck" => {
            let image = single_image_arg(args)?;
            let report = fsck_image(&image).map_err(|error| error.to_string())?;
            print_image_report(&image, &report);
            println!("fsck: clean");
        }
        "dump-super" => {
            let image = single_image_arg(args)?;
            let superblock = dump_superblock(&image).map_err(|error| error.to_string())?;
            print_superblock(&superblock);
        }
        "stat" => {
            let image = args.next().ok_or_else(usage)?;
            let path = args.next().unwrap_or_else(|| "/".to_string());
            if args.next().is_some() {
                return Err(usage());
            }
            let report = stat_image(&image, &path).map_err(|error| error.to_string())?;
            print_image_report(&image, &report.image);
            println!("path: {path}");
            println!("inode: {}", report.inode.id.raw());
            println!("kind: {:?}", report.inode.kind);
            println!("size: {}", report.inode.size);
            println!("mode: {:o}", report.inode.permissions.bits());
            println!("links: {}", report.inode.links);
            println!("object_id: {}", report.object_id);
            println!("path_identity: {:#x}", report.path_identity);
            println!("metadata_flags: {:#x}", report.metadata_flags);
            println!("service_class: {}", report.service_class);
            println!("extent_map_version: {}", report.extent_map_version);
            println!("extent_count: {}", report.extent_count);
            println!("signature_len: {}", report.signature_len);
            println!("capability_len: {}", report.capability_len);
            println!("last_transaction_id: {}", report.last_transaction_id);
            println!("mutation_state: {}", report.mutation_state);
        }
        "help" | "--help" | "-h" => {
            println!("{}", usage());
        }
        _ => return Err(usage()),
    }
    Ok(())
}

fn single_image_arg(mut args: impl Iterator<Item = String>) -> Result<String, String> {
    let image = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }
    Ok(image)
}

fn parse_optional_sectors(mut args: impl Iterator<Item = String>) -> Result<u64, String> {
    let Some(flag) = args.next() else {
        return Ok(DEFAULT_SECTORS);
    };
    if flag != "--sectors" {
        return Err(usage());
    }
    let sectors = args
        .next()
        .ok_or_else(usage)?
        .parse::<u64>()
        .map_err(|_| "--sectors must be an unsigned integer".to_string())?;
    if args.next().is_some() {
        return Err(usage());
    }
    Ok(sectors)
}

fn print_image_report(image: &str, report: &QfsImageReport) {
    println!("image: {image}");
    print_superblock(&report.superblock);
    println!("cached_books: {}", report.cached_books);
    println!(
        "cached_book_index_entries: {}",
        report.cached_book_index_entries
    );
    println!(
        "cached_chapter_index_entries: {}",
        report.cached_chapter_index_entries
    );
    println!("cached_inode_records: {}", report.cached_inode_records);
    println!("cached_journal_records: {}", report.cached_journal_records);
}

fn print_superblock(superblock: &QfsSuperblock) {
    println!("magic: {:?}", superblock.magic);
    println!("version: {}", superblock.version);
    println!("sector_size: {}", superblock.sector_size);
    println!("page_sectors: {}", superblock.page_sectors);
    println!("book_pages: {}", superblock.book_pages);
    println!("total_books: {}", superblock.total_books);
    println!("root_inode: {}", superblock.root_inode);
    println!(
        "inode_table: book={} page={}",
        superblock.inode_table.book_id, superblock.inode_table.page
    );
    println!(
        "journal: book={} page={}",
        superblock.journal.book_id, superblock.journal.page
    );
    println!(
        "free_space_bitmap: book={} page={}",
        superblock.free_space_bitmap.book_id, superblock.free_space_bitmap.page
    );
    println!("flags: {}", superblock.flags);
    println!("total_sectors: {}", superblock.total_sectors);
    println!("free_sectors: {}", superblock.free_sectors);
}

fn usage() -> String {
    "usage:\n  qfsprogs mkfs <image> [--sectors N]\n  qfsprogs fsck <image>\n  qfsprogs dump-super <image>\n  qfsprogs stat <image> [path]".to_string()
}
