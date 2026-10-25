use std::cell::RefCell;
use tje_mem::{Address, Bus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordAccess {
    Read(u32, u16),
    Write(u32, u16),
}

struct WordBus {
    value: u16,
    reload_override: Option<u16>,
    accesses: RefCell<Vec<WordAccess>>,
}

impl Bus for WordBus {
    fn read_u8(&self, _: u32) -> u8 {
        panic!("the compiled sequence uses word reads, not byte reads");
    }

    fn write_u8(&mut self, _: u32, _: u8) {
        panic!("the compiled sequence uses a word write, not byte writes");
    }

    fn read_u16(&self, address: u32) -> u16 {
        let mut accesses = self.accesses.borrow_mut();
        let value = if accesses.len() == 2 {
            self.reload_override.unwrap_or(self.value)
        } else {
            self.value
        };
        accesses.push(WordAccess::Read(address, value));
        value
    }

    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .get_mut()
            .push(WordAccess::Write(address, value));
        self.value = value;
    }
}

#[test]
fn increment_and_reload_keeps_three_word_accesses_for_every_word() {
    let address = 0xff7ff8;
    let mut bus = WordBus {
        value: 0,
        reload_override: None,
        accesses: RefCell::new(Vec::with_capacity(3)),
    };
    for old in 0..=u16::MAX {
        bus.value = old;
        bus.accesses.get_mut().clear();
        let next = old.wrapping_add(1);
        let result = super::increment_word_and_reload(&mut bus, Address::new(address));
        assert_eq!(result, next as i16, "input {old:#06x}");
        assert_eq!(
            bus.accesses.borrow().as_slice(),
            &[
                WordAccess::Read(address, old),
                WordAccess::Write(address, next),
                WordAccess::Read(address, next),
            ],
            "input {old:#06x}",
        );
    }
}

#[test]
fn increment_and_reload_consumes_the_reload_not_the_computed_value() {
    let address = 0xff7ff8;
    let mut bus = WordBus {
        value: 8,
        reload_override: Some(42),
        accesses: RefCell::new(Vec::with_capacity(3)),
    };
    assert_eq!(
        super::increment_word_and_reload(&mut bus, Address::new(address)),
        42,
    );
    assert_eq!(
        bus.accesses.borrow().as_slice(),
        &[
            WordAccess::Read(address, 8),
            WordAccess::Write(address, 9),
            WordAccess::Read(address, 42),
        ],
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearAccess {
    DiscardByte(u32, u8),
    DiscardWord(u32, u16),
    WriteByte(u32, u8),
    WriteWord(u32, u16),
}

struct ClearBus {
    value: u16,
    fail_discard: bool,
    accesses: Vec<ClearAccess>,
}

impl Bus for ClearBus {
    fn read_u8(&self, _: u32) -> u8 {
        panic!("CLR's pre-read must be tagged Discarded, never semantic");
    }

    fn read_u16(&self, _: u32) -> u16 {
        panic!("CLR's pre-read must be tagged Discarded, never semantic");
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses.push(ClearAccess::WriteByte(address, value));
        self.value = (self.value & 0xff00) | u16::from(value);
    }

    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses.push(ClearAccess::WriteWord(address, value));
        self.value = value;
    }
}

impl tje_mem::Platform for ClearBus {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("CLR RAM instruction must not access the VDP");
    }

    fn write_vdp_data(&mut self, _: u16) {
        panic!("CLR RAM instruction must not access the VDP");
    }
}

impl crate::Platform for ClearBus {
    fn discard_u8(&mut self, address: u32) -> crate::Result<()> {
        self.accesses
            .push(ClearAccess::DiscardByte(address, self.value as u8));
        if self.fail_discard {
            return Err(crate::Error::Platform("controlled discard failure".into()));
        }
        Ok(())
    }

    fn discard_u16(&mut self, address: u32) -> crate::Result<()> {
        self.accesses
            .push(ClearAccess::DiscardWord(address, self.value));
        if self.fail_discard {
            return Err(crate::Error::Platform("controlled discard failure".into()));
        }
        Ok(())
    }

    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("CLR RAM instruction must not use a semantic device read");
    }

    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("CLR RAM instruction must not use a semantic device read");
    }

    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("CLR RAM instruction must not use a device write");
    }

    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("CLR RAM instruction must not use a device write");
    }
}

#[test]
fn byte_clear_keeps_a_tagged_byte_preread_before_zero_for_every_byte() {
    let address = 0xff7ff9;
    let mut bus = ClearBus {
        value: 0,
        fail_discard: false,
        accesses: Vec::with_capacity(2),
    };
    for old in 0..=u8::MAX {
        bus.value = 0xa500 | u16::from(old);
        bus.accesses.clear();
        super::clear_byte_instruction(&mut bus, address).unwrap();
        assert_eq!(
            bus.accesses,
            [
                ClearAccess::DiscardByte(address, old),
                ClearAccess::WriteByte(address, 0),
            ]
        );
        assert_eq!(bus.value, 0xa500);
    }
}

#[test]
fn word_clear_keeps_one_tagged_word_preread_before_zero_for_every_word() {
    let address = 0xff7ff8;
    let mut bus = ClearBus {
        value: 0,
        fail_discard: false,
        accesses: Vec::with_capacity(2),
    };
    for old in 0..=u16::MAX {
        bus.value = old;
        bus.accesses.clear();
        super::clear_word_instruction(&mut bus, address).unwrap();
        assert_eq!(
            bus.accesses,
            [
                ClearAccess::DiscardWord(address, old),
                ClearAccess::WriteWord(address, 0),
            ]
        );
        assert_eq!(bus.value, 0);
    }
}

#[test]
fn failed_discard_does_not_issue_the_clear_store() {
    let address = 0xff7ff8;
    let mut bus = ClearBus {
        value: 0xabcd,
        fail_discard: true,
        accesses: Vec::with_capacity(1),
    };
    assert!(super::clear_byte_instruction(&mut bus, address).is_err());
    assert_eq!(bus.accesses, [ClearAccess::DiscardByte(address, 0xcd)]);
    assert_eq!(bus.value, 0xabcd);
    bus.accesses.clear();
    assert!(super::clear_word_instruction(&mut bus, address).is_err());
    assert_eq!(bus.accesses, [ClearAccess::DiscardWord(address, 0xabcd)]);
    assert_eq!(bus.value, 0xabcd);
}
