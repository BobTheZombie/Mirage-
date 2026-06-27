use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gen-device-db: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--check") => {
            let root = args
                .next()
                .map(PathBuf::from)
                .ok_or("missing descriptor root after --check")?;
            if args.next().is_some() {
                return Err("unexpected trailing arguments".into());
            }
            gen_device_db::load_database(&root).map(|_| ())
        }
        Some("--out") => {
            let out = args
                .next()
                .map(PathBuf::from)
                .ok_or("missing output path after --out")?;
            if args.next().is_some() {
                return Err("unexpected trailing arguments".into());
            }
            let db = gen_device_db::load_database(PathBuf::from("devices/db"))?;
            gen_device_db::write_generated(&db, &out)
        }
        _ => Err("usage: gen-device-db --check <devices/db> | --out <generated.rs>".into()),
    }
}
