use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"MBRTFS\0\x01";
const HEADER_SIZE: usize = 64;
const PATH_CAP: usize = 64;
const ENTRY_SIZE: usize = 128;
const MAX_FILES: usize = 16;

struct Args {
    tree: PathBuf,
    image: PathBuf,
    name: String,
    entry: String,
}
struct FileEntry {
    path: String,
    bytes: Vec<u8>,
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let mut files = Vec::new();
    collect_files(&args.tree, &args.tree, &mut files).map_err(|e| e.to_string())?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(format!("runtime image must contain 1..={MAX_FILES} files"));
    }
    let image = build_image(&args, &files)?;
    if let Some(parent) = args.image.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&args.image, &image).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} bytes, {} files)",
        args.image.display(),
        image.len(),
        files.len()
    );
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut name = String::from("spider-rt");
    let mut entry = String::from("/sbin/spider-rs");
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => name = it.next().ok_or("--name requires a value")?,
            "--entry" => entry = it.next().ok_or("--entry requires a value")?,
            _ => positional.push(arg),
        }
    }
    if positional.len() != 2 {
        return Err("usage: mk-runtime-image <tree> <image> [--name NAME] [--entry PATH]".into());
    }
    Ok(Args {
        tree: positional[0].clone().into(),
        image: positional[1].clone().into(),
        name,
        entry,
    })
}

fn collect_files(root: &Path, current: &Path, out: &mut Vec<FileEntry>) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_files(root, &path, out)?;
        } else if ty.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            out.push(FileEntry {
                path: format!("/{rel}"),
                bytes: fs::read(&path)?,
            });
        }
    }
    Ok(())
}

fn build_image(args: &Args, files: &[FileEntry]) -> Result<Vec<u8>, String> {
    let entries_offset = HEADER_SIZE + PATH_CAP;
    let data_offset = entries_offset + files.len() * ENTRY_SIZE;
    let mut image = vec![0u8; data_offset];
    image[0..8].copy_from_slice(MAGIC);
    put_u32(&mut image, 8, files.len() as u32);
    put_u32(&mut image, 12, entries_offset as u32);
    put_fixed(&mut image, 20, 32, &args.name, "name")?;
    put_fixed(&mut image, 52, 16, "0", "version")?;
    put_fixed(&mut image, HEADER_SIZE, PATH_CAP, &args.entry, "entry")?;
    image[16] = args.name.len().min(32) as u8;
    image[17] = 1;
    image[18] = args.entry.len().min(PATH_CAP) as u8;

    let mut offset = data_offset;
    for (idx, file) in files.iter().enumerate() {
        if file.path.len() > PATH_CAP {
            return Err(format!("path too long: {}", file.path));
        }
        let entry_offset = entries_offset + idx * ENTRY_SIZE;
        image[entry_offset] = file.path.len() as u8;
        image[entry_offset + 1] = u8::from(file.path == args.entry);
        put_u32(&mut image, entry_offset + 4, offset as u32);
        put_u32(&mut image, entry_offset + 8, file.bytes.len() as u32);
        put_u32(&mut image, entry_offset + 12, crc32(&file.bytes));
        image[entry_offset + 16..entry_offset + 16 + file.path.len()]
            .copy_from_slice(file.path.as_bytes());
        image.extend_from_slice(&file.bytes);
        offset += file.bytes.len();
    }
    Ok(image)
}

fn put_fixed(
    image: &mut [u8],
    offset: usize,
    cap: usize,
    value: &str,
    label: &str,
) -> Result<(), String> {
    if value.len() > cap {
        return Err(format!("{label} too long"));
    }
    image[offset..offset + value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}

fn put_u32(image: &mut [u8], offset: usize, value: u32) {
    image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
