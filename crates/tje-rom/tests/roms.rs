//! Optional private inputs are loaded at runtime, never embedded in the test binary.

use std::fs;
use std::io::{Cursor, ErrorKind, Write};
use std::path::Path;

use tje_rom::{Revision, Rom};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

fn check_revision(revision: Revision, filename: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(filename);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!(
                "SKIP {revision}: {} is absent; supply your private ROM to exercise this test",
                path.display(),
            );
            return;
        }
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    };
    let imported = Rom::import(&bytes).unwrap_or_else(|error| panic!("{revision}: {error}"));
    assert_eq!(imported.revision(), revision);
    assert_eq!(imported.bytes().len(), revision.rom_size());
    assert_eq!(imported.bytes(), bytes);
    assert_eq!(imported.into_bytes(), bytes);

    let mut swapped = bytes.clone();
    for pair in swapped.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    assert_eq!(Rom::import(&swapped).unwrap().bytes(), bytes);

    let mut smd = vec![0; 512 + bytes.len()];
    smd[8..11].copy_from_slice(&[0xaa, 0xbb, 0x06]);
    for (input, output) in bytes
        .as_chunks::<{ 16 * 1024 }>()
        .0
        .iter()
        .zip(smd[512..].as_chunks_mut::<{ 16 * 1024 }>().0)
    {
        for (index, pair) in input.as_chunks::<2>().0.iter().enumerate() {
            output[index] = pair[1];
            output[8 * 1024 + index] = pair[0];
        }
    }
    assert_eq!(Rom::import(&smd).unwrap().bytes(), bytes);

    for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
        for (name, payload) in [("game.bin", &bytes), ("game.smd", &smd)] {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(
                    name,
                    SimpleFileOptions::default().compression_method(method),
                )
                .unwrap();
            writer.write_all(payload).unwrap();
            let archive = writer.finish().unwrap().into_inner();
            let imported = Rom::import(&archive).unwrap();
            assert_eq!(imported.revision(), revision);
            assert_eq!(imported.bytes(), bytes);
        }
    }
    eprintln!("PASS {revision}: raw, byte-swapped, SMD, stored ZIP and DEFLATE ZIP");
}

#[test]
fn rev02_private_rom() {
    check_revision(Revision::Rev02, "rev02.bin");
}

#[test]
fn rev00_private_rom() {
    check_revision(Revision::Rev00, "rev00.bin");
}
