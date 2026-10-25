use std::{
    fs::File,
    io::{self, Read, Write},
};
use tje_maker::{MAX_BODY_BYTES, MAX_CODE_BYTES, ValidatedCourse};

fn read_bounded(path: &str, limit: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(format!("{path}: input exceeds {limit} bytes").into());
    }
    Ok(bytes)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err(
            "usage: tje-maker validate|encode COURSE.tjemaker.json | decode CODE.txt".into(),
        );
    }
    match args[1].as_str() {
        "validate" => {
            let course = ValidatedCourse::from_json(&read_bounded(&args[2], MAX_BODY_BYTES)?)?;
            println!(
                "valid: {}\nsha256: {}\ncanonical bytes: {}",
                course.course().title,
                course.digest_hex(),
                course.canonical_json().len()
            );
        }
        "encode" => {
            let course = ValidatedCourse::from_json(&read_bounded(&args[2], MAX_BODY_BYTES)?)?;
            let code = course.to_share_code()?;
            if code.len() > 8192 {
                eprintln!(
                    "share code is {} bytes; prefer a local file instead of pasting it",
                    code.len()
                );
            }
            println!("{code}");
        }
        "decode" => {
            let bytes = read_bounded(&args[2], MAX_CODE_BYTES + 2)?;
            let text = std::str::from_utf8(&bytes)?.trim_end_matches(['\r', '\n']);
            let course = ValidatedCourse::from_share_code(text)?;
            io::stdout().lock().write_all(course.canonical_json())?;
            println!();
        }
        _ => {
            return Err(
                "expected validate, encode or decode; no network operation is supported".into(),
            );
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
