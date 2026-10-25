//! Genesis memory map and 68000 + Z80 bus interfaces

use bincode::{Decode, Encode};
use genesis_config::GenesisEmulatorConfig;
use jgenesis_common::boxedarray::{BoxedByteArray, BoxedWordArray};
use jgenesis_common::cheats::CheatWordOverrides;
use jgenesis_common::num::{GetBit, U16Ext};

const MAIN_RAM_LEN_WORDS: usize = 64 * 1024 / 2;
const AUDIO_RAM_LEN: usize = 8 * 1024;

type RamCheatOverrides = CheatWordOverrides<0xFF0000, 0xFFFFFF>;

#[derive(Debug, Clone, Encode, Decode)]
pub struct Memory {
    main_ram: BoxedWordArray<MAIN_RAM_LEN_WORDS>,
    audio_ram: BoxedByteArray<AUDIO_RAM_LEN>,
    ram_cheat_overrides: RamCheatOverrides,
    #[cfg(feature = "reference-proof-hooks")]
    audio_fixture_watch: Option<AudioFixtureWatch>,
}

/// TJE reference fixture audit: one actual physical byte commit, including aliases
/// and same-value writes. No address other than the fixed mailbox lock is watched.
#[cfg(feature = "reference-proof-hooks")]
#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct AudioFixtureByteWrite {
    pub sequence: u64,
    pub before: u8,
    pub value: u8,
}

#[cfg(feature = "reference-proof-hooks")]
#[derive(Debug, Clone, Encode, Decode)]
struct AudioFixtureWatch {
    sequence: u64,
    opaque_access: bool,
    writes: std::collections::VecDeque<AudioFixtureByteWrite>,
}

impl Memory {
    /// TJE strict data-plane control view: retain audio RAM and cheat policy by
    /// borrow, excluding only physical main RAM for a separately checked domain.
    pub fn hybrid_control_without_main_ram(&self) -> impl Encode + '_ {
        #[cfg(not(feature = "reference-proof-hooks"))]
        {
            let Self { main_ram: _, audio_ram, ram_cheat_overrides } = self;
            (audio_ram, ram_cheat_overrides)
        }
        #[cfg(feature = "reference-proof-hooks")]
        {
            let Self { main_ram: _, audio_ram, ram_cheat_overrides, audio_fixture_watch } = self;
            (audio_ram, ram_cheat_overrides, audio_fixture_watch)
        }
    }

    /// Physical storage, not cheat-substituted logical reads. Inspection only.
    pub fn hybrid_raw_main_ram(&self) -> &[u16] {
        self.main_ram.as_ref()
    }

    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn new(config: &GenesisEmulatorConfig) -> Self {
        Self {
            main_ram: BoxedWordArray::new(),
            audio_ram: BoxedByteArray::new(),
            ram_cheat_overrides: RamCheatOverrides::new(&config.cheat_codes),
            #[cfg(feature = "reference-proof-hooks")]
            audio_fixture_watch: None,
        }
    }

    #[must_use]
    pub fn read_main_ram_byte(&self, address: u32) -> u8 {
        if let Some(cheat) = self.ram_cheat_overrides.get(address) {
            return cheat.to_be_bytes()[(address & 1) as usize];
        }

        let word = self.main_ram[((address & 0xFFFF) >> 1) as usize];
        word.to_be_bytes()[(address & 1) as usize]
    }

    #[must_use]
    pub fn read_main_ram<const WORD: bool>(&self, address: u32) -> u16 {
        if WORD {
            self.read_main_ram_word(address)
        } else {
            self.read_main_ram_byte(address).into()
        }
    }

    #[must_use]
    pub fn read_main_ram_word(&self, address: u32) -> u16 {
        if let Some(cheat) = self.ram_cheat_overrides.get(address) {
            return cheat;
        }

        self.main_ram[((address & 0xFFFF) >> 1) as usize]
    }

    pub fn write_main_ram<const WORD: bool>(&mut self, address: u32, value: u16) {
        if WORD {
            self.write_main_ram_word(address, value);
        } else {
            self.write_main_ram_byte(address, value as u8);
        }
    }

    pub fn write_main_ram_byte(&mut self, address: u32, value: u8) {
        let word = &mut self.main_ram[((address & 0xFFFF) >> 1) as usize];
        if !address.bit(0) {
            word.set_msb(value);
        } else {
            word.set_lsb(value);
        }
    }

    pub fn write_main_ram_word(&mut self, address: u32, value: u16) {
        self.main_ram[((address & 0xFFFF) >> 1) as usize] = value;
    }

    #[must_use]
    pub fn read_audio_ram(&self, address: u16) -> u8 {
        self.audio_ram[(address & 0x1FFF) as usize]
    }

    pub fn write_audio_ram(&mut self, address: u16, value: u8) {
        #[cfg(feature = "reference-proof-hooks")]
        if address & 0x1fff == 0x41
            && let Some(watch) = &mut self.audio_fixture_watch
        {
            watch.sequence = watch.sequence.saturating_add(1);
            watch.writes.push_back(AudioFixtureByteWrite {
                sequence: watch.sequence,
                before: self.audio_ram[0x41],
                value,
            });
        }
        self.audio_ram[(address & 0x1FFF) as usize] = value;
    }

    /// TJE development-only writer lease. A mutable debug view invalidates it.
    #[cfg(feature = "reference-proof-hooks")]
    pub fn begin_audio_fixture_watch(&mut self) -> bool {
        if self.audio_fixture_watch.is_some() {
            return false;
        }
        self.audio_fixture_watch = Some(AudioFixtureWatch {
            sequence: 0,
            opaque_access: false,
            writes: std::collections::VecDeque::with_capacity(4),
        });
        true
    }

    #[cfg(feature = "reference-proof-hooks")]
    pub fn audio_fixture_watch_status(&self) -> Option<(u64, bool)> {
        self.audio_fixture_watch.as_ref().map(|watch| (watch.sequence, watch.opaque_access))
    }

    #[cfg(feature = "reference-proof-hooks")]
    pub fn pop_audio_fixture_write(&mut self) -> Option<AudioFixtureByteWrite> {
        self.audio_fixture_watch.as_mut()?.writes.pop_front()
    }

    #[cfg(feature = "reference-proof-hooks")]
    pub fn finish_audio_fixture_watch(&mut self) {
        self.audio_fixture_watch = None;
    }

    pub fn reload_config(&mut self, config: &GenesisEmulatorConfig) {
        self.ram_cheat_overrides.update_cheat_codes(&config.cheat_codes);
    }

    #[must_use]
    pub fn debug_ram_view(&mut self) -> (&mut [u16], &mut [u8]) {
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(watch) = &mut self.audio_fixture_watch {
            // A borrowed mutable slice cannot report individual writes. Refuse
            // an attestation rather than silently allowing an unobserved writer.
            watch.opaque_access = true;
        }
        (self.main_ram.as_mut_slice(), self.audio_ram.as_mut_slice())
    }
}

#[cfg(test)]
mod hybrid_control_tests {
    use super::*;

    fn control(memory: &Memory) -> Vec<u8> {
        bincode::encode_to_vec(
            memory.hybrid_control_without_main_ram(),
            bincode::config::standard(),
        )
        .unwrap()
    }

    #[test]
    fn control_view_keeps_audio_ram_and_cheats_but_not_physical_main_ram() {
        let mut memory = Memory::new(&GenesisEmulatorConfig::default());
        let before = bincode::encode_to_vec(&memory, bincode::config::standard()).unwrap();
        let baseline = control(&memory);
        assert_eq!(before, bincode::encode_to_vec(&memory, bincode::config::standard()).unwrap());
        memory.write_main_ram_word(0xff0010, 0xabcd);
        assert_eq!(control(&memory), baseline);
        assert_eq!(memory.hybrid_raw_main_ram()[8], 0xabcd);
        memory.write_audio_ram(0x17, 0x5a);
        assert_ne!(control(&memory), baseline);
        let audio = control(&memory);
        memory.reload_config(&GenesisEmulatorConfig {
            cheat_codes: vec![(0xff0010, 0x1234)],
            ..GenesisEmulatorConfig::default()
        });
        assert_ne!(control(&memory), audio);
        assert_eq!(memory.read_main_ram_word(0xff0010), 0x1234);
        assert_eq!(memory.hybrid_raw_main_ram()[8], 0xabcd);
    }

    #[test]
    fn encoding_and_raw_borrow_preserve_every_encoded_memory_byte() {
        let mut memory = Memory::new(&GenesisEmulatorConfig {
            cheat_codes: vec![(0xff0010, 0x1234)],
            ..GenesisEmulatorConfig::default()
        });
        memory.write_main_ram_word(0xff0010, 0xabcd);
        memory.write_audio_ram(3, 7);
        let before = bincode::encode_to_vec(&memory, bincode::config::standard()).unwrap();
        for _ in 0..4 {
            assert!(!control(&memory).is_empty());
            assert_eq!(memory.hybrid_raw_main_ram()[8], 0xabcd);
        }
        assert_eq!(before, bincode::encode_to_vec(&memory, bincode::config::standard()).unwrap());
    }
}
