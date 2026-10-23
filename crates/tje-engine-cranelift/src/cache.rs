//! The cache contains private, locally generated machine code, never distributable assets.
//!
//! A cache is trusted only as much as the current user's data directory. SHA-256 detects
//! corruption, not a malicious writer who can replace both code and checksum. No cache is
//! loaded from the working directory, ROM directory, network, or another user's directory.
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"TJENAT\0\x01";
const MAX_CACHE_BYTES: u64 = 1 << 30;
pub(crate) const CODE_ALIGNMENT: usize = 16;

/// Bump this when changing lowering semantics, the callback ABI, or cache format.
pub const ENGINE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-clif137-abi1");

/// Every input that can change executable code is checked before loading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub rom_sha1: [u8; 20],
    pub program_sha256: [u8; 32],
    pub target: String,
}

impl Identity {
    pub fn file_name(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(ENGINE_VERSION);
        hash.update(self.program_sha256);
        hash.update(&self.target);
        format!(
            "{}-{ENGINE_VERSION}-{:x}.tjec",
            hex(&self.rom_sha1),
            hash.finalize()
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        write!(result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

/// Native cache location. `data_dir` is the same application-data root accepted
/// by the desktop frontend; the backend always adds its own `engine-native` child.
#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub data_dir: PathBuf,
}

impl CacheConfig {
    /// Resolve the frontend's per-user local application-data directory.
    pub fn for_current_user() -> Result<Self> {
        let directories =
            directories::ProjectDirs::from("org", "toejam-earl-port", "ToeJam & Earl")
                .ok_or(Error::NoUserDataDirectory)?;
        Ok(Self {
            data_dir: directories.data_local_dir().to_path_buf(),
        })
    }

    pub(crate) fn path(&self, identity: &Identity) -> PathBuf {
        self.data_dir
            .join("engine-native")
            .join(identity.file_name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub pc: u32,
    pub offset: u32,
    pub length: u32,
}

pub(crate) struct Artifact {
    pub entries: Vec<Entry>,
    pub code: Vec<u8>,
}

impl Artifact {
    pub fn validate(&self, expected_pcs: impl ExactSizeIterator<Item = u32>) -> Result<()> {
        if self.entries.len() != expected_pcs.len() {
            return Err(Error::InvalidCache("continuation count"));
        }
        let mut end = 0;
        for (entry, expected_pc) in self.entries.iter().zip(expected_pcs) {
            if entry.pc != expected_pc || entry.pc & 1 != 0 || entry.pc > 0x00ff_ffff {
                return Err(Error::InvalidCache("continuation address"));
            }
            let offset = entry.offset as usize;
            if offset < end || !offset.is_multiple_of(CODE_ALIGNMENT) || entry.length == 0 {
                return Err(Error::InvalidCache("continuation layout"));
            }
            end = offset
                .checked_add(entry.length as usize)
                .filter(|&end| end <= self.code.len())
                .ok_or(Error::InvalidCache("continuation bounds"))?;
        }
        if self.entries.is_empty() || end != self.code.len() {
            return Err(Error::InvalidCache("empty or trailing code"));
        }
        Ok(())
    }
}

/// Validate identity, checksum and all lengths before returning any executable bytes.
pub(crate) fn load(path: &Path, expected: &Identity) -> Result<Artifact> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_CACHE_BYTES {
        return Err(Error::InvalidCache("cache too large"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.len() < 32 {
        return Err(Error::InvalidCache("truncated checksum"));
    }
    let (payload, checksum) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(payload).as_slice() != checksum {
        return Err(Error::InvalidCache("checksum"));
    }
    let mut cursor = Cursor(payload);
    if cursor.take(8)? != MAGIC {
        return Err(Error::InvalidCache("format version"));
    }
    if cursor.take(20)? != expected.rom_sha1 || cursor.take(32)? != expected.program_sha256 {
        return Err(Error::InvalidCache("ROM or IR identity"));
    }
    if cursor.string()? != ENGINE_VERSION || cursor.string()? != expected.target {
        return Err(Error::InvalidCache("engine or target identity"));
    }
    let count = cursor.u32()? as usize;
    let code_length = cursor.u32()? as usize;
    let entries_length = count
        .checked_mul(12)
        .ok_or(Error::InvalidCache("entry length"))?;
    if entries_length.checked_add(code_length) != Some(cursor.0.len()) {
        return Err(Error::InvalidCache("payload length"));
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(Entry {
            pc: cursor.u32()?,
            offset: cursor.u32()?,
            length: cursor.u32()?,
        });
    }
    Ok(Artifact {
        entries,
        code: cursor.take(code_length)?.to_vec(),
    })
}

/// Atomic publication prevents readers from seeing partially written code. Concurrent
/// first launches may both compile, but publish complete, identically keyed artifacts.
pub(crate) fn store(path: &Path, identity: &Identity, artifact: &Artifact) -> Result<u64> {
    let directory = path
        .parent()
        .ok_or(Error::InvalidCache("cache directory"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    let mut hash = Sha256::new();
    {
        let mut output = HashedWriter {
            file: &mut file,
            hash: &mut hash,
        };
        output.write_all(MAGIC)?;
        output.write_all(&identity.rom_sha1)?;
        output.write_all(&identity.program_sha256)?;
        write_string(&mut output, ENGINE_VERSION)?;
        write_string(&mut output, &identity.target)?;
        let count = u32::try_from(artifact.entries.len()).map_err(|_| Error::CodeTooLarge)?;
        let length = u32::try_from(artifact.code.len()).map_err(|_| Error::CodeTooLarge)?;
        output.write_all(&count.to_le_bytes())?;
        output.write_all(&length.to_le_bytes())?;
        for entry in &artifact.entries {
            output.write_all(&entry.pc.to_le_bytes())?;
            output.write_all(&entry.offset.to_le_bytes())?;
            output.write_all(&entry.length.to_le_bytes())?;
        }
        output.write_all(&artifact.code)?;
    }
    file.write_all(&hash.finalize())?;
    file.as_file().sync_all()?;
    let length = file.as_file().metadata()?.len();
    file.persist(path).map_err(|error| Error::Io(error.error))?;
    Ok(length)
}

fn write_string(writer: &mut impl Write, text: &str) -> Result<()> {
    let length = u32::try_from(text.len()).map_err(|_| Error::CodeTooLarge)?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(text.as_bytes())?;
    Ok(())
}

struct HashedWriter<'a> {
    file: &'a mut tempfile::NamedTempFile,
    hash: &'a mut Sha256,
}

impl Write for HashedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self.file.write(bytes)?;
        self.hash.update(&bytes[..length]);
        Ok(length)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let result = self
            .0
            .get(..length)
            .ok_or(Error::InvalidCache("truncated payload"))?;
        self.0 = &self.0[length..];
        Ok(result)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four-byte slice"),
        ))
    }
    fn string(&mut self) -> Result<&'a str> {
        let length = self.u32()? as usize;
        std::str::from_utf8(self.take(length)?)
            .map_err(|_| Error::InvalidCache("identity encoding"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            rom_sha1: [3; 20],
            program_sha256: [7; 32],
            target: "synthetic-test".into(),
        }
    }

    #[test]
    fn round_trip_checks_identity_and_checksum() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cache");
        let artifact = Artifact {
            entries: vec![Entry {
                pc: 0x100,
                offset: 0,
                length: 3,
            }],
            code: vec![1, 2, 3],
        };
        let expected = identity();
        store(&path, &expected, &artifact).unwrap();
        let loaded = load(&path, &expected).unwrap();
        loaded.validate([0x100].into_iter()).unwrap();
        assert_eq!(loaded.code, artifact.code);
        assert_eq!(loaded.entries, artifact.entries);
        let mut wrong = expected.clone();
        wrong.rom_sha1[0] ^= 1;
        assert!(load(&path, &wrong).is_err());
        wrong = expected.clone();
        wrong.program_sha256[0] ^= 1;
        assert!(load(&path, &wrong).is_err());
        wrong = expected.clone();
        wrong.target.push('x');
        assert!(load(&path, &wrong).is_err());
        let mut bytes = fs::read(&path).unwrap();
        bytes[8] ^= 1;
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            load(&path, &expected),
            Err(Error::InvalidCache("checksum"))
        ));
    }

    #[test]
    fn rejects_truncated_and_overlapping_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cache");
        let artifact = Artifact {
            entries: vec![Entry {
                pc: 0x100,
                offset: 0,
                length: 2,
            }],
            code: vec![1, 2],
        };
        store(&path, &identity(), &artifact).unwrap();
        let bytes = fs::read(&path).unwrap();
        for length in 0..bytes.len() {
            fs::write(&path, &bytes[..length]).unwrap();
            assert!(load(&path, &identity()).is_err(), "truncation at {length}");
        }
        let malformed = Artifact {
            entries: vec![
                Entry {
                    pc: 0x100,
                    offset: 0,
                    length: 17,
                },
                Entry {
                    pc: 0x102,
                    offset: 16,
                    length: 1,
                },
            ],
            code: vec![0; 17],
        };
        assert!(malformed.validate([0x100, 0x102].into_iter()).is_err());
        assert!(artifact.validate([0x102].into_iter()).is_err());
        assert!(artifact.validate([0x100, 0x102].into_iter()).is_err());
    }
}
