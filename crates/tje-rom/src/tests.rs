use std::io::Write;

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use super::*;

fn synthetic_image(blocks: usize) -> Vec<u8> {
    let mut image: Vec<u8> = (0..SMD_BLOCK_SIZE * blocks)
        .map(|index| (index.wrapping_mul(37) ^ (index >> 5)) as u8)
        .collect();
    image[0x100..0x104].copy_from_slice(b"SEGA");
    image[0x120..0x180].fill(b' ');
    let name = b"SYNTHETIC TEST IMAGE";
    image[0x150..0x150 + name.len()].copy_from_slice(name);
    image
}

fn swapped(image: &[u8]) -> Vec<u8> {
    let mut result = image.to_vec();
    for pair in result.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    result
}

fn interleaved(image: &[u8], magic: bool) -> Vec<u8> {
    let complete_blocks = image.len().is_multiple_of(SMD_BLOCK_SIZE);
    assert!(complete_blocks);
    let mut result = vec![0; SMD_HEADER_SIZE + image.len()];
    if magic {
        result[8..11].copy_from_slice(&[0xaa, 0xbb, 0x06]);
    }
    for (input, output) in image.as_chunks::<SMD_BLOCK_SIZE>().0.iter().zip(
        result[SMD_HEADER_SIZE..]
            .as_chunks_mut::<SMD_BLOCK_SIZE>()
            .0,
    ) {
        for (index, pair) in input.as_chunks::<2>().0.iter().enumerate() {
            output[index] = pair[1];
            output[SMD_HALF_BLOCK + index] = pair[0];
        }
    }
    result
}

fn archive(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(method);
    for &(name, bytes) in entries {
        if name.ends_with('/') {
            writer.add_directory(name, options).unwrap();
        } else {
            writer.start_file(name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

fn unknown(image: &[u8]) -> ImportError {
    ImportError::UnknownRom {
        size: image.len(),
        header_name: "SYNTHETIC TEST IMAGE".to_owned(),
        sha1: format!("{:x}", Sha1::digest(image)),
    }
}

#[test]
fn revisions_match_only_the_pinned_checksums() {
    assert_eq!(identify_digest(&REV02_SHA1), Some(Revision::Rev02));
    assert_eq!(identify_digest(&REV00_SHA1), Some(Revision::Rev00));
    assert_eq!(identify_digest(&[0; 20]), None);
    for revision in Revision::ALL {
        assert_eq!(revision.rom_size(), ROM_SIZE);
        assert_eq!(revision.sha1().len(), 40);
        assert_eq!(revision.to_string(), revision.name());
    }
}

#[test]
fn raw_bin_md_and_gen_bytes_are_unchanged() {
    let image = synthetic_image(1);
    let normalized = normalize_image(Cow::Borrowed(&image)).unwrap();
    assert!(matches!(normalized, Cow::Borrowed(_)));
    assert_eq!(normalized.as_ref(), image);
    assert_eq!(Rom::import(&image).unwrap_err(), unknown(&image));
}

#[test]
fn unknown_rom_reports_size_name_and_checksum() {
    let image = synthetic_image(1);
    let error = Rom::import(&image).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(&image.len().to_string()));
    assert!(message.contains("SYNTHETIC TEST IMAGE"));
    assert!(message.contains(&format!("{:x}", Sha1::digest(&image))));
    assert!(message.contains("SHA-1 checksum"));
    assert!(message.contains("REV 02"));
    assert!(message.contains("REV 00"));
}

#[test]
fn empty_and_short_images_are_diagnostic_errors() {
    for size in [0, 1, 4, 8, 10, 0x100, 0x103, 0x17f] {
        let image = vec![b' '; size];
        assert_eq!(
            Rom::import(&image).unwrap_err(),
            ImportError::UnknownRom {
                size,
                header_name: "<unavailable>".to_owned(),
                sha1: format!("{:x}", Sha1::digest(&image)),
            },
        );
    }
}

#[test]
fn missing_sega_header_is_not_accepted() {
    let mut image = synthetic_image(1);
    image[0x100..0x104].copy_from_slice(b"NOPE");
    assert_eq!(Rom::import(&image).unwrap_err(), unknown(&image));
}

#[test]
fn header_names_fall_back_and_escape_nonprintable_bytes() {
    let mut image = synthetic_image(1);
    image[0x150..0x180].fill(b' ');
    image[0x120..0x128].copy_from_slice(b"DOMESTIC");
    assert_eq!(header_name(&image), "DOMESTIC");
    image[0x120..0x180].fill(b' ');
    assert_eq!(header_name(&image), "<empty>");
    image[0x150..0x154].copy_from_slice(&[b'A', 0, 0xff, b'B']);
    assert_eq!(header_name(&image), "A\u{fffd}\u{fffd}B");
    assert!(!Rom::import(&image).unwrap_err().to_string().contains('\0'));
}

#[test]
fn swapped_image_is_normalized_before_identification() {
    let image = synthetic_image(1);
    let input = swapped(&image);
    assert_eq!(normalize_image(Cow::Borrowed(&input)).unwrap(), image);
    assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
}

#[test]
fn owned_swapped_image_reuses_its_allocation() {
    let mut input = swapped(&synthetic_image(1));
    let pointer = input.as_mut_ptr();
    let output = normalize_image(Cow::Owned(input)).unwrap();
    assert_eq!(output.as_ptr(), pointer);
}

#[test]
fn odd_byte_swapped_image_is_rejected() {
    let mut input = swapped(&synthetic_image(1));
    input.push(0);
    let error = Rom::import(&input).unwrap_err();
    assert_eq!(error, ImportError::InvalidByteSwap { size: input.len() });
    assert!(error.to_string().contains("16-bit words"));
}

#[test]
fn smd_magic_and_multiple_blocks_are_deinterleaved() {
    for blocks in [1, 3] {
        let image = synthetic_image(blocks);
        let input = interleaved(&image, true);
        assert_eq!(normalize_image(Cow::Borrowed(&input)).unwrap(), image);
        assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
    }
}

#[test]
fn smd_without_magic_is_detected_from_the_interleaved_sega_header() {
    let image = synthetic_image(2);
    let input = interleaved(&image, false);
    assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
}

#[test]
fn smd_with_swapped_words_is_normalized() {
    let image = synthetic_image(1);
    for magic in [false, true] {
        let input = interleaved(&swapped(&image), magic);
        assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
    }
}

#[test]
fn smd_missing_or_partial_payload_is_rejected() {
    let input = interleaved(&synthetic_image(1), true);
    for size in [11, SMD_HEADER_SIZE, input.len() - 1] {
        let error = Rom::import(&input[..size]).unwrap_err();
        assert_eq!(error, ImportError::InvalidSmd { size });
        assert!(error.to_string().contains("interleaved blocks"));
    }
    let no_magic = interleaved(&synthetic_image(1), false);
    assert!(matches!(
        Rom::import(&no_magic[..no_magic.len() - 1]),
        Err(ImportError::InvalidSmd { .. }),
    ));
}

#[test]
fn raw_header_takes_precedence_over_coincidental_smd_magic() {
    let mut image = synthetic_image(1);
    image[8..11].copy_from_slice(&[0xaa, 0xbb, 0x06]);
    assert_eq!(Rom::import(&image).unwrap_err(), unknown(&image));
}

#[test]
fn stored_and_deflated_zip_members_are_read() {
    let image = synthetic_image(1);
    for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
        let input = archive(&[("game.bin", &image)], method);
        assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
    }
}

#[test]
fn zip_allows_directories_companions_and_case_insensitive_rom_suffixes() {
    let image = synthetic_image(1);
    for extension in ["BIN", "md", "GeN", "SMD"] {
        let name = format!("folder/game.{extension}");
        let input = archive(
            &[
                ("folder/", &[]),
                ("notes.txt", b"Not a ROM"),
                (&name, &image),
            ],
            CompressionMethod::Deflated,
        );
        assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
    }
}

#[test]
fn zip_smd_and_swapped_images_are_normalized() {
    let image = synthetic_image(1);
    for payload in [interleaved(&image, true), swapped(&image)] {
        let input = archive(&[("game.smd", &payload)], CompressionMethod::Deflated);
        assert_eq!(Rom::import(&input).unwrap_err(), unknown(&image));
    }
}

#[test]
fn zip_requires_exactly_one_rom_member() {
    let image = synthetic_image(1);
    for (entries, count) in [
        (vec![], 0),
        (vec![("notes.txt", b"Not a ROM".as_slice())], 0),
        (vec![("bin", image.as_slice())], 0),
        (vec![("nested.zip", image.as_slice())], 0),
        (vec![("game.bin/", b"".as_slice())], 0),
        (vec![("one.bin", image.as_slice()), ("two.gen", &image)], 2),
    ] {
        let input = archive(&entries, CompressionMethod::Stored);
        let error = Rom::import(&input).unwrap_err();
        assert_eq!(error, ImportError::ArchiveRomCount { count });
        assert!(error.to_string().contains("exactly one"));
    }
}

#[test]
fn zip_signature_with_invalid_structure_is_rejected() {
    let mut input = Vec::from(*b"PK");
    input.extend([3, 4]);
    input.extend_from_slice(b"not a ZIP archive");
    let error = Rom::import(&input).unwrap_err();
    assert!(matches!(error, ImportError::InvalidArchive { .. }));
    assert!(error.to_string().contains("ROM ZIP"));
}

#[test]
fn zip_member_size_is_bounded_before_decompression() {
    let payload = vec![0; MAX_ARCHIVE_IMAGE_SIZE + 1];
    let input = archive(&[("game.bin", &payload)], CompressionMethod::Deflated);
    let error = Rom::import(&input).unwrap_err();
    assert_eq!(
        error,
        ImportError::ArchiveImageTooLarge {
            size: payload.len() as u64,
        },
    );
    assert!(error.to_string().contains("too large"));
}

#[test]
fn zip_member_crc_failure_is_not_ignored() {
    let image = synthetic_image(1);
    let mut input = archive(&[("game.bin", &image)], CompressionMethod::Stored);
    let offset = {
        let mut reader = zip::ZipArchive::new(Cursor::new(&input)).unwrap();
        reader.by_index(0).unwrap().data_start() as usize
    };
    input[offset] ^= 0xff;
    assert!(matches!(
        Rom::import(&input),
        Err(ImportError::InvalidArchive { .. }),
    ));
}
