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
