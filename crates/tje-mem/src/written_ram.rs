//! Written-byte bookkeeping for readable stack ownership and causal proofs.
//!
//! This is data only: it grants no read permission or execution authority.
//! Addresses use the same physical 64-KiB work RAM through every 68000 mirror.
use crate::{ADDRESS_MASK, RAM_SIZE, RAM_START};

const WORDS: usize = RAM_SIZE / 64;
const DIRTY_WORDS: usize = WORDS / 64;

pub fn canonical_ram_byte(address: u32) -> Option<u32> {
    let address = address & ADDRESS_MASK;
    (address >= 0xe00000).then_some(RAM_START | (address & 0xffff))
}

/// A reusable bitmap. Reset visits only words touched since the last reset.
/// Fixed bookkeeping makes reads, writes and reset allocation-free.
pub struct WrittenRam {
    words: [u64; WORDS],
    dirty: [u64; DIRTY_WORDS],
    touched: [u16; WORDS],
    touched_len: usize,
}

impl Default for WrittenRam {
    fn default() -> Self {
        Self {
            words: [0; WORDS],
            dirty: [0; DIRTY_WORDS],
            touched: [0; WORDS],
            touched_len: 0,
        }
    }
}

impl WrittenRam {
    pub fn get(&self, address: u32) -> bool {
        let Some(address) = canonical_ram_byte(address) else {
            return false;
        };
        let index = (address & 0xffff) as usize;
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    pub fn set(&mut self, address: u32) {
        let Some(address) = canonical_ram_byte(address) else {
            return;
        };
        let index = (address & 0xffff) as usize;
        let word = index / 64;
        let dirty = 1 << (word % 64);
        if self.dirty[word / 64] & dirty == 0 {
            self.dirty[word / 64] |= dirty;
            self.touched[self.touched_len] = word as u16;
            self.touched_len += 1;
        }
        self.words[word] |= 1 << (index % 64);
    }

    /// Clear a half-open RAM/mirror range, including exclusive 01000000.
    /// Clearing does not forget that a word needs a later root reset.
    pub fn clear(&mut self, low: u32, high: u32) {
        assert!(low <= high);
        if low == high {
            return;
        }
        let length = (high - low) as usize;
        assert!(length <= RAM_SIZE && canonical_ram_byte(high - 1).is_some());
        let low = (canonical_ram_byte(low).expect("RAM clear range") & 0xffff) as usize;
        let high = low + length;
        self.clear_offsets(low, high.min(RAM_SIZE));
        if high > RAM_SIZE {
            self.clear_offsets(0, high - RAM_SIZE);
        }
    }

    fn clear_offsets(&mut self, low: usize, high: usize) {
        for word in low / 64..=(high - 1) / 64 {
            let start = low.saturating_sub(word * 64).min(64);
            let end = (high - word * 64).min(64);
            self.words[word] &= !((u64::MAX << start) & (u64::MAX >> (64 - end)));
        }
    }

    pub fn reset(&mut self) {
        for &index in &self.touched[..self.touched_len] {
            let word = usize::from(index);
            self.words[word] = 0;
            self.dirty[word / 64] &= !(1 << (word % 64));
        }
        self.touched_len = 0;
    }

    pub fn addresses(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(word, &bits)| {
            let mut remaining = bits;
            std::iter::from_fn(move || {
                if remaining == 0 {
                    return None;
                }
                let bit = remaining.trailing_zeros();
                remaining &= remaining - 1;
                Some(RAM_START + word as u32 * 64 + bit)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_share_one_byte_without_aliasing_cartridge_addresses() {
        let mut written = WrittenRam::default();
        written.set(0xe01234);
        assert!(written.get(0xff1234));
        assert!(written.get(0x1ff1234));
        assert!(!written.get(0x1234));
        written.set(0x1234);
        assert_eq!(written.addresses().collect::<Vec<_>>(), [0xff1234]);
    }

    #[test]
    fn allocation_clear_does_not_duplicate_dirty_words_or_leave_old_history() {
        let mut written = WrittenRam::default();
        for _ in 0..1000 {
            written.set(0xff003f);
            written.set(0xff0040);
            written.clear(0xff003f, 0xff0041);
        }
        assert_eq!(written.touched_len, 2);
        assert_eq!(written.addresses().count(), 0);
        written.set(0xff0040);
        written.reset();
        assert!(!written.get(0xff0040));
        assert_eq!(written.touched_len, 0);
        written.set(0xff0040);
        assert_eq!(written.touched_len, 1);
    }

    #[test]
    fn range_clear_preserves_both_neighbours_and_the_full_ram_endpoint() {
        let mut written = WrittenRam::default();
        for address in RAM_START..RAM_START + RAM_SIZE as u32 {
            written.set(address);
        }
        written.clear(0xff003f, 0xff0081);
        assert!(written.get(0xff003e));
        assert!(!written.get(0xff003f));
        assert!(!written.get(0xff0080));
        assert!(written.get(0xff0081));
        written.clear(0xfffffe, 0x1000000);
        assert!(!written.get(0xffffff));
        written.reset();
        assert_eq!(written.addresses().count(), 0);
        assert_eq!(written.touched_len, 0);
    }
}

#[cfg(test)]
mod mirror_clear_tests {
    use super::*;
    #[test]
    fn mirrored_clear_wraps_only_the_physical_ram_page() {
        let mut written = WrittenRam::default();
        written.set(0xfffffe);
        written.set(0xff0000);
        written.set(0xff0001);
        written.clear(0xe0fffe, 0xe10001);
        assert!(!written.get(0xfffffe));
        assert!(!written.get(0xff0000));
        assert!(written.get(0xff0001));
        written.set(0xfffffe);
        assert_eq!(written.touched_len, 2);
        written.reset();
        assert_eq!(written.addresses().count(), 0);
    }
}
