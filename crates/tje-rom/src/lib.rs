//! Import a user's ToeJam & Earl cartridge without filesystem or platform APIs.
//!
//! [`Rom::import`] accepts raw `.bin`/`.md`/`.gen` bytes, word-byte-swapped images,
//! interleaved Super Magic Drive (`.smd`) images, or a ZIP containing exactly one
//! supported ROM member. ZIP directories and non-ROM companion files are ignored;
//! stored and DEFLATE members are supported with a pure-Rust backend. Nothing is
//! extracted to disk. The returned bytes always have the original cartridge order.
//!
//! Only the two pinned whole-ROM SHA-1 checksums are accepted. No copyrighted game
//! data is included in this crate, including its synthetic tests. This API also
//! builds for `wasm32-unknown-unknown`; a browser can pass file-picker bytes directly.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::fmt;
use std::io::{Cursor, Read};

use sha1::{Digest, Sha1};

const ROM_SIZE: usize = 1024 * 1024;
const SMD_HEADER_SIZE: usize = 512;
const SMD_BLOCK_SIZE: usize = 16 * 1024;
const SMD_HALF_BLOCK: usize = SMD_BLOCK_SIZE / 2;
const MAX_ARCHIVE_IMAGE_SIZE: usize = ROM_SIZE + SMD_HEADER_SIZE;
const REV02_SHA1: [u8; 20] = [
    0x85, 0xe8, 0xd0, 0xa4, 0xfa, 0xc5, 0x91, 0xb2, 0x5b, 0x77, 0xc3, 0x56, 0x80, 0xac, 0x41, 0x75,
    0x97, 0x6f, 0x25, 0x1b,
];
const REV00_SHA1: [u8; 20] = [
    0x7f, 0x82, 0xd8, 0xb5, 0x7f, 0xff, 0x88, 0xbd, 0xca, 0x5d, 0x8a, 0xff, 0x85, 0xb0, 0x1e, 0x23,
    0x1d, 0xc1, 0x23, 0x9a,
];

/// An original retail revision, identified by the complete normalized image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Revision {
    Rev02,
    Rev00,
}

impl Revision {
    pub const ALL: [Self; 2] = [Self::Rev02, Self::Rev00];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Rev02 => "REV 02",
            Self::Rev00 => "REV 00",
        }
    }

    pub const fn sha1(self) -> &'static str {
        match self {
            Self::Rev02 => "85e8d0a4fac591b25b77c35680ac4175976f251b",
            Self::Rev00 => "7f82d8b57fff88bdca5d8aff85b01e231dc1239a",
        }
    }

    pub const fn rom_size(self) -> usize {
        ROM_SIZE
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A verified cartridge image, in big-endian 68000 byte order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rom {
    bytes: Vec<u8>,
    revision: Revision,
}

impl Rom {
    /// Normalize a supported image/container, then check its whole-image SHA-1.
    ///
    /// File extensions are unnecessary for raw inputs. Within a ZIP, the single
    /// ROM member must have a `.bin`, `.md`, `.gen`, or `.smd` extension (case
    /// insensitive). Its uncompressed size is bounded before allocation, including
    /// the optional SMD copier header. Nested archives are not ROM members.
    pub fn import(bytes: &[u8]) -> Result<Self, ImportError> {
        let image = if is_zip(bytes) {
            Cow::Owned(read_archive(bytes)?)
        } else {
            Cow::Borrowed(bytes)
        };
        let image = normalize_image(image)?;
        let digest = Sha1::digest(image.as_ref());
        let revision = identify_digest(&digest).ok_or_else(|| ImportError::UnknownRom {
            size: image.len(),
            header_name: header_name(&image),
            sha1: format!("{digest:x}"),
        })?;
        Ok(Self {
            bytes: image.into_owned(),
            revision,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Import failures never silently accept an unrecognized or damaged cartridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportError {
    /// The normalized image does not match either supported whole-ROM checksum.
    UnknownRom {
        size: usize,
        header_name: String,
        sha1: String,
    },
    InvalidSmd {
        size: usize,
    },
    InvalidByteSwap {
        size: usize,
    },
    InvalidArchive {
        reason: String,
    },
    ArchiveRomCount {
        count: usize,
    },
    ArchiveImageTooLarge {
        size: u64,
    },
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRom {
                size,
                header_name,
                sha1,
            } => write!(
                f,
                "unrecognized ToeJam & Earl ROM: {size} bytes, header name {header_name:?}, SHA-1 checksum {sha1}; expected REV 02 ({}) or REV 00 ({})",
                Revision::Rev02.sha1(),
                Revision::Rev00.sha1(),
            ),
            Self::InvalidSmd { size } => write!(
                f,
                "invalid SMD image: {size} bytes; expected a 512-byte copier header followed by complete 16384-byte interleaved blocks",
            ),
            Self::InvalidByteSwap { size } => write!(
                f,
                "invalid byte-swapped ROM: {size} bytes is not a whole number of 16-bit words",
            ),
            Self::InvalidArchive { reason } => {
                write!(f, "invalid or unsupported ROM ZIP: {reason}")
            }
            Self::ArchiveRomCount { count } => write!(
                f,
                "ROM ZIP must contain exactly one .bin/.md/.gen/.smd member; found {count}",
            ),
            Self::ArchiveImageTooLarge { size } => write!(
                f,
                "ROM ZIP member is too large: {size} bytes; maximum is {MAX_ARCHIVE_IMAGE_SIZE} bytes including an SMD header",
            ),
        }
    }
}

impl std::error::Error for ImportError {}

fn identify_digest(digest: &[u8]) -> Option<Revision> {
    if digest == REV02_SHA1 {
        Some(Revision::Rev02)
    } else if digest == REV00_SHA1 {
        Some(Revision::Rev00)
    } else {
        None
    }
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}

fn archive_error(error: impl fmt::Display) -> ImportError {
    ImportError::InvalidArchive {
        reason: error.to_string(),
    }
}

fn read_archive(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(archive_error)?;
    let mut selected = None;
    let mut count = 0;
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(archive_error)?;
        let extension = file
            .name()
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension);
        if !file.is_dir()
            && ["bin", "md", "gen", "smd"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            count += 1;
            selected = Some(index);
        }
    }
    if count != 1 {
        return Err(ImportError::ArchiveRomCount { count });
    }
    // `selected` is Some precisely when the count is one.
    let mut file = archive
        .by_index(selected.expect("one ROM member was selected"))
        .map_err(archive_error)?;
    if file.size() > MAX_ARCHIVE_IMAGE_SIZE as u64 {
        return Err(ImportError::ArchiveImageTooLarge { size: file.size() });
    }
    let mut image = Vec::with_capacity(file.size() as usize);
    // Also bound actual output: never trust only an archive's size declaration.
    file.by_ref()
        .take(MAX_ARCHIVE_IMAGE_SIZE as u64 + 1)
        .read_to_end(&mut image)
        .map_err(archive_error)?;
    if image.len() > MAX_ARCHIVE_IMAGE_SIZE {
        return Err(ImportError::ArchiveImageTooLarge {
            size: image.len() as u64,
        });
    }
    Ok(image)
}

fn smd_header(bytes: &[u8]) -> bool {
    bytes.get(8..11) == Some(&[0xaa, 0xbb, 0x06])
}

fn smd_system_name(bytes: &[u8]) -> Option<[u8; 4]> {
    let payload = bytes.get(SMD_HEADER_SIZE..)?;
    Some([
        *payload.get(SMD_HALF_BLOCK + 0x80)?,
        *payload.get(0x80)?,
        *payload.get(SMD_HALF_BLOCK + 0x81)?,
        *payload.get(0x81)?,
    ])
}

fn normalize_image(mut image: Cow<'_, [u8]>) -> Result<Cow<'_, [u8]>, ImportError> {
    let system_name = image.get(0x100..0x104);
    let raw_header = matches!(system_name, Some(b"SEGA" | b"ESAG"));
    if !raw_header
        && (smd_header(&image)
            || smd_system_name(&image).is_some_and(|name| name == *b"SEGA" || name == *b"ESAG"))
    {
        if image.len() <= SMD_HEADER_SIZE
            || !(image.len() - SMD_HEADER_SIZE).is_multiple_of(SMD_BLOCK_SIZE)
        {
            return Err(ImportError::InvalidSmd { size: image.len() });
        }
        let payload = &image[SMD_HEADER_SIZE..];
        let mut decoded = Vec::with_capacity(payload.len());
        for block in payload.as_chunks::<SMD_BLOCK_SIZE>().0 {
            let (odd, even) = block.split_at(SMD_HALF_BLOCK);
            for (&even, &odd) in even.iter().zip(odd) {
                decoded.push(even);
                decoded.push(odd);
            }
        }
        image = Cow::Owned(decoded);
    }
    if image.get(0x100..0x104) == Some(b"ESAG") {
        if !image.len().is_multiple_of(2) {
            return Err(ImportError::InvalidByteSwap { size: image.len() });
        }
        for pair in image.to_mut().as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
    }
    Ok(image)
}

fn header_name(image: &[u8]) -> String {
    for range in [0x150..0x180, 0x120..0x150] {
        if let Some(field) = image.get(range) {
            let field = field.trim_ascii();
            if !field.is_empty() {
                return field
                    .iter()
                    .map(|&byte| {
                        if byte.is_ascii_graphic() || byte == b' ' {
                            char::from(byte)
                        } else {
                            '\u{fffd}'
                        }
                    })
                    .collect();
            }
        }
    }
    if image.len() < 0x180 {
        "<unavailable>".to_owned()
    } else {
        "<empty>".to_owned()
    }
}

#[cfg(test)]
mod tests;
