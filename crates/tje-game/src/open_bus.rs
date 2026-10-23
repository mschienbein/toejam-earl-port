//! Source-backed 68000 open-bus operands for translated device reads.
//!
//! Some devices drive only part of a bus word. If software observes the remaining
//! bits, the translation must identify the original instruction's prefetched ROM
//! word, not provide a literal value or borrow an unrelated entry-time latch.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenBusError {
    SourceRequired { address: u32, word: bool },
    InvalidSource { address: u32 },
    MissingRomWord { address: u32 },
    InvalidDevice { address: u32, word: bool },
    RomUnavailable,
    UnsupportedPlatform,
}

impl fmt::Display for OpenBusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SourceRequired { address, word } => write!(
                formatter,
                "{} MMIO read {address:06X} requires an explicit ROM prefetch source",
                if word { "word" } else { "byte" }
            ),
            Self::InvalidSource { address } => write!(
                formatter,
                "open-bus prefetch source is not an aligned ROM word: {address:08X}"
            ),
            Self::MissingRomWord { address } => write!(
                formatter,
                "open-bus prefetch source {address:06X} is outside the ROM"
            ),
            Self::InvalidDevice { address, word } => write!(
                formatter,
                "invalid {} MMIO access {address:06X}",
                if word { "word" } else { "byte" }
            ),
            Self::RomUnavailable => {
                formatter.write_str("an open-bus source requires the user's ROM")
            }
            Self::UnsupportedPlatform => {
                formatter.write_str("platform has no explicit MMIO operand contract")
            }
        }
    }
}

impl std::error::Error for OpenBusError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenBusSource {
    prefetch_address: u32,
}

impl OpenBusSource {
    /// The address comes from the matching original instruction/code layout for
    /// the selected revision. This is not a cycle charge or an arbitrary ROM read.
    pub const fn prefetch(address: u32) -> Self {
        Self {
            prefetch_address: address,
        }
    }

    pub fn address(self) -> Result<u32, OpenBusError> {
        let address = self.prefetch_address & tje_mem::ADDRESS_MASK;
        if address & 1 != 0 || address > 0x3ffffe {
            return Err(OpenBusError::InvalidSource {
                address: self.prefetch_address,
            });
        }
        Ok(address)
    }

    /// Native platforms resolve the same operand from the user's immutable ROM.
    /// Missing bytes are an error, never a zero/ready/open-bus fallback.
    pub fn word_from_rom(self, rom: &[u8]) -> Result<u16, OpenBusError> {
        let address = self.address()?;
        let index = address as usize;
        let bytes = rom
            .get(index..index + 2)
            .ok_or(OpenBusError::MissingRomWord { address })?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_operand_is_the_actual_big_endian_rom_word() {
        let rom = [0x12, 0x34, 0xab, 0xcd];
        assert_eq!(
            OpenBusSource::prefetch(0).word_from_rom(&rom).unwrap(),
            0x1234
        );
        assert_eq!(
            OpenBusSource::prefetch(2).word_from_rom(&rom).unwrap(),
            0xabcd
        );
        assert_eq!(
            OpenBusSource::prefetch(0xff000002)
                .word_from_rom(&rom)
                .unwrap(),
            0xabcd
        );
    }

    #[test]
    fn invalid_or_missing_sources_do_not_fabricate_an_operand() {
        for source in [1, 0xa04000, 0xff8000] {
            assert!(
                OpenBusSource::prefetch(source)
                    .word_from_rom(&[0; 4])
                    .is_err()
            );
        }
        assert!(OpenBusSource::prefetch(4).word_from_rom(&[0; 4]).is_err());
    }
}
