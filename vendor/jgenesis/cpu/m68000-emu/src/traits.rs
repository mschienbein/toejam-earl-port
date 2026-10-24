use crate::debug::M68000Debugger;

/// TJE hybrid observation: distinguish consumed data from real bus cycles whose
/// values are discarded. Every purpose still performs the underlying bus access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bincode", derive(bincode::Encode, bincode::Decode))]
pub enum ReadPurpose {
    #[default]
    Semantic,
    Discarded,
    Prefetch,
}

/// TJE check observation at a successful architectural exception boundary.
/// No register/RAM payload is copied into ordinary bus accesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "bincode", derive(bincode::Encode, bincode::Decode))]
pub enum ExceptionBoundary {
    Entered { vector: u32, handler: u32, depth: u32 },
    Returned { depth: u32 },
}

pub trait BusInterface {
    // Addresses are 32-bit internally but the 68000 only has a 24-bit address bus
    const ADDRESS_MASK: u32 = 0x00FF_FFFF;

    type DebugView<'a>: M68000Debugger
    where
        Self: 'a;

    fn read_byte(&mut self, address: u32) -> u8;

    fn read_word(&mut self, address: u32) -> u16;

    #[inline]
    fn read_byte_tagged(&mut self, address: u32, _purpose: ReadPurpose) -> u8 {
        self.read_byte(address)
    }

    #[inline]
    fn read_word_tagged(&mut self, address: u32, _purpose: ReadPurpose) -> u16 {
        self.read_word(address)
    }

    fn write_byte(&mut self, address: u32, value: u8);

    fn write_word(&mut self, address: u32, value: u16);

    fn read_long_word(&mut self, address: u32) -> u32 {
        let high_word = self.read_word_tagged(address, ReadPurpose::Semantic);
        let low_word = self.read_word_tagged(address.wrapping_add(2), ReadPurpose::Semantic);

        (u32::from(high_word) << 16) | u32::from(low_word)
    }

    fn write_long_word(&mut self, address: u32, value: u32) {
        let high_word = (value >> 16) as u16;
        let low_word = value as u16;

        self.write_word(address, high_word);
        self.write_word(address.wrapping_add(2), low_word);
    }

    // Auto-vectored interrupt level; should be between 0 and 7, with 0 indicating no interrupt
    fn interrupt_level(&self) -> u8;

    fn acknowledge_interrupt(&mut self, interrupt_level: u8);

    /// Defaults to no code after monomorphization for unobserved buses.
    #[inline]
    fn observe_exception_boundary(&mut self, _boundary: ExceptionBoundary) {}

    fn halt(&self) -> bool;

    fn reset(&self) -> bool;

    fn debug_view(&mut self) -> Option<Self::DebugView<'_>> {
        None
    }
}
