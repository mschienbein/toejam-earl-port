//! Pure source/protocol tests, not a ROM timing proof or an emulator substitute.
use super::*;
use std::collections::BTreeMap;

fn initial_registers() -> Registers {
    Registers {
        data: std::array::from_fn(|index| 0x1234_5678_u32.wrapping_mul(index as u32 + 1)),
        address: std::array::from_fn(|index| 0x8765_4321_u32.wrapping_mul(index as u32 + 1)),
        user_stack: 0xff7600,
        supervisor_stack: 0xff7000,
        status: 0x271f,
    }
}

#[derive(Default)]
struct Memory {
    bytes: BTreeMap<u32, u8>,
    effects: Vec<Observation>,
    roles: Vec<u16>,
    ccrs: Vec<(u16, u16)>,
}

impl Memory {
    fn byte(&self, address: u32) -> u8 {
        *self
            .bytes
            .get(&address)
            .expect("explicitly initialized test byte")
    }

    fn word(&self, address: u32) -> u16 {
        (u16::from(self.byte(address)) << 8) | u16::from(self.byte(address.wrapping_add(1)))
    }

    fn put_byte(&mut self, address: u32, value: u8) {
        self.bytes.insert(address, value);
    }

    fn put_word(&mut self, address: u32, value: u16) {
        self.put_byte(address, (value >> 8) as u8);
        self.put_byte(address.wrapping_add(1), value as u8);
    }

    fn put_long(&mut self, address: u32, value: u32) {
        self.put_word(address, (value >> 16) as u16);
        self.put_word(address.wrapping_add(2), value as u16);
    }

    fn actual(&mut self, effect: Effect) -> Observation {
        let value = match effect.kind {
            EffectKind::Write(value) => {
                match effect.width {
                    Width::Byte => self.put_byte(effect.address, value as u8),
                    Width::Word => self.put_word(effect.address, value),
                }
                value
            }
            EffectKind::Read(_) => match effect.width {
                Width::Byte if matches!(effect.address, 0xa10003 | 0xa10005) => {
                    // Explicit pad fixture: the two samples depend on the real
                    // source's preceding TH writes, not a precomputed result.
                    if self.byte(effect.address) & 0x40 == 0 {
                        0x30
                    } else {
                        0x15
                    }
                }
                Width::Byte => u16::from(self.byte(effect.address)),
                Width::Word => self.word(effect.address),
            },
        };
        let observation = Observation { effect, value };
        self.effects.push(observation);
        observation
    }
}

fn site(location: Location) -> InstructionSite {
    // Pure protocol tests do not bind or claim a ROM instruction address.
    InstructionSite {
        location,
        address: 0,
        fallthrough: 0,
    }
}

fn run(source: &mut Source, mut registers: Registers, memory: &mut Memory) -> Commit {
    let mut location = source.program().entry;
    for _ in 0..200_000 {
        memory.roles.push(location.role);
        source.begin_instruction(site(location), registers).unwrap();
        while let Some(effect) = source.next_effect().unwrap() {
            source.acknowledge(memory.actual(effect)).unwrap();
        }
        let commit = source.finish_instruction().unwrap();
        memory.ccrs.push((location.role, commit.registers.status));
        match commit.path {
            Path::Next(next) => {
                location = next;
                registers = commit.registers;
            }
            Path::Return { .. } => return commit,
            Path::Call { .. } => panic!("a declared platform leaf emitted a callee path"),
        }
    }
    panic!("source fixture did not return within its diagnostic instruction ceiling");
}

fn fixture(registers: &Registers) -> Memory {
    let mut memory = Memory::default();
    memory.put_long(registers.stack_pointer(), 0x0012_3456);
    memory
}

fn first_ccr(memory: &Memory, role: u16) -> u16 {
    memory
        .ccrs
        .iter()
        .find_map(|&(at, status)| (at == role).then_some(status))
        .expect("fixture exercised this source role")
}

#[test]
fn controller_roles_keep_both_delays_samples_sound_order_and_register_residue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for leaf in [Leaf::Controller1, Leaf::Controller2] {
            for delay in [0, 1, 2, 0x80, 0x81, 0xff] {
                let registers = initial_registers();
                let entry = registers.clone();
                let mut memory = fixture(&registers);
                memory.put_byte(Z80_DAC_DELAY, delay);
                memory.put_byte(Z80_PSG_ACTIVE, 0x5a);
                let mirror = ram::g_psg_effect_status(revision).address();
                memory.put_word(mirror, 0xa5bb);
                let mut source = Source::new(revision, leaf);
                let result = run(&mut source, registers, &mut memory);
                let Path::Return { address, value } = result.path else {
                    panic!("expected RTS")
                };
                assert_eq!(address, 0x0012_3456);
                assert_eq!(value, Some(0x2a));
                assert_eq!(result.registers.data[0], 0x2a);
                assert_eq!(
                    result.registers.data[1],
                    (entry.data[1] & 0xffff_0000) | 0xff15
                );
                assert_eq!(&result.registers.data[2..], &entry.data[2..]);
                assert_eq!(result.registers.address, entry.address);
                assert_eq!(result.registers.user_stack, entry.user_stack);
                assert_eq!(
                    result.registers.supervisor_stack,
                    entry.supervisor_stack + 4
                );
                assert_eq!(
                    result.registers.status,
                    0x2704 | if delay < 2 { 0x10 } else { 0 }
                );
                assert_eq!(memory.byte(Z80_DAC_DELAY), delay.wrapping_sub(2));
                assert_eq!(memory.word(mirror), 0x5abb);
                assert_eq!(memory.roles.iter().filter(|&&role| role == 4).count(), 11);
                assert_eq!(memory.roles.iter().filter(|&&role| role == 11).count(), 11);
                assert_eq!(memory.roles.len(), 40);
                assert_eq!(memory.effects.len(), 13);
                assert_eq!(first_ccr(&memory, 4), 0x2710);
                assert_eq!(first_ccr(&memory, 11), 0x2700);
                let subtraction = match delay {
                    0 | 1 => 0x2719,
                    2 => 0x2704,
                    0x80 | 0x81 => 0x2702,
                    0xff => 0x2708,
                    _ => unreachable!("fixed DAC-delay fixtures"),
                };
                assert_eq!(first_ccr(&memory, 16), subtraction);
                assert_eq!(
                    first_ccr(&memory, 17),
                    0x2700 | if delay < 2 { 0x10 } else { 0 }
                );
                assert!(!memory.effects.iter().any(|observation| {
                    observation.effect.address == Z80_BUS_REQUEST
                        && matches!(observation.effect.kind, EffectKind::Read(_))
                }));
                assert!(
                    source
                        .begin_instruction(site(source.program().entry), entry)
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn word_stream_keeps_zero_count_wrap_live_argument_reads_and_exact_rts() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for count in [1_u32, 3, 0x1234_0001, 0x8000, 0] {
            let registers = initial_registers();
            let entry = registers.clone();
            let mut memory = fixture(&registers);
            let source_address = 0x0040_0000;
            let words = if count as u16 == 0 {
                65_536
            } else {
                u32::from(count as u16)
            };
            memory.put_long(entry.stack_pointer() + 4, source_address);
            memory.put_long(entry.stack_pointer() + 8, count);
            for index in 0..words {
                memory.put_word(
                    source_address + index * 2,
                    if index + 1 == words {
                        0x8000
                    } else {
                        index as u16
                    },
                );
            }
            let mut source = Source::new(revision, Leaf::WordStream);
            let result = run(&mut source, registers, &mut memory);
            assert_eq!(
                result.path,
                Path::Return {
                    address: 0x0012_3456,
                    value: None
                }
            );
            assert_eq!(result.registers.data[0], (count & 0xffff_0000) | 0xffff);
            assert_eq!(&result.registers.data[1..], &entry.data[1..]);
            assert_eq!(result.registers.address[0], source_address + words * 2);
            assert_eq!(&result.registers.address[1..], &entry.address[1..]);
            assert_eq!(
                result.registers.status,
                0x2708 | if count as u16 == 0 { 0x10 } else { 0 }
            );
            assert_eq!(
                result.registers.supervisor_stack,
                entry.supervisor_stack + 4
            );
            assert_eq!(memory.roles.len(), words as usize * 2 + 4);
            let subtraction = match count as u16 {
                0 => 0x2719,
                1 => 0x2704,
                0x8000 => 0x2702,
                _ => 0x2700,
            };
            assert_eq!(first_ccr(&memory, 2), subtraction);
            assert_eq!(
                memory
                    .effects
                    .iter()
                    .filter(|event| {
                        event.effect.address == VDP_DATA
                            && matches!(event.effect.kind, EffectKind::Write(_))
                    })
                    .count(),
                words as usize
            );
        }
    }
}

#[test]
fn palette_roles_mask_slot_keep_command_high_word_and_copy_exactly_sixteen() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for slot in [0_u32, 1, 3, u32::MAX] {
            let registers = initial_registers();
            let entry = registers.clone();
            let mut memory = fixture(&registers);
            let source_address = 0xff9000;
            memory.put_long(entry.stack_pointer() + 4, source_address);
            memory.put_long(entry.stack_pointer() + 8, slot);
            for index in 0..16 {
                memory.put_word(
                    source_address + index * 2,
                    if index == 15 { 0x8000 } else { index as u16 },
                );
            }
            let mut source = Source::new(revision, Leaf::Palette);
            let result = run(&mut source, registers, &mut memory);
            let command = (((slot & 3) << 5) | 0xc000) << 16;
            assert_eq!(
                result.path,
                Path::Return {
                    address: 0x0012_3456,
                    value: None
                }
            );
            assert_eq!(result.registers.data[0], command | 0xffff);
            assert_eq!(&result.registers.data[1..], &entry.data[1..]);
            assert_eq!(result.registers.address[0], source_address + 32);
            assert_eq!(result.registers.status, 0x2708);
            assert_eq!(first_ccr(&memory, 6), 0x2708);
            assert_eq!(first_ccr(&memory, 7), 0x2700);
            let port_writes: Vec<_> = memory
                .effects
                .iter()
                .filter(|event| {
                    matches!(event.effect.address, VDP_CONTROL | 0xc00006)
                        && matches!(event.effect.kind, EffectKind::Write(_))
                })
                .map(|event| (event.effect.address, event.value))
                .collect();
            assert_eq!(
                port_writes,
                [(VDP_CONTROL, (command >> 16) as u16), (0xc00006, 0)]
            );
            assert_eq!(memory.roles.iter().filter(|&&role| role == 8).count(), 16);
            assert_eq!(memory.roles.iter().filter(|&&role| role == 9).count(), 16);
        }
    }
}

#[test]
fn dma_roles_use_real_save_bytes_lazy_slots_ram_publication_and_discarded_restore_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut registers = initial_registers();
        registers.data[2] = 0;
        registers.data[4] = 0x8000_0000;
        let entry = registers.clone();
        let sp = entry.stack_pointer();
        let mut memory = fixture(&registers);
        let source_address = 0x00ff_b8da;
        let destination = 0xc002;
        memory.put_long(sp + 4, source_address);
        memory.put_long(sp + 8, destination);
        memory.put_long(sp + 12, 0x1234_0000);
        memory.put_byte(ram::g_mode2_reg_softcopy(revision).address(), 0x60);
        let mut source = Source::new(revision, Leaf::Dma);
        let result = run(&mut source, registers, &mut memory);
        let command = 0x4002_0083;
        assert_eq!(
            result.path,
            Path::Return {
                address: 0x0012_3456,
                value: None
            }
        );
        assert_eq!(result.registers.data[0], command);
        assert_eq!(result.registers.data[1], 0x4002_0080);
        assert_eq!(&result.registers.data[2..], &entry.data[2..]);
        assert_eq!(result.registers.address, entry.address);
        assert_eq!(result.registers.supervisor_stack, sp + 4);
        assert_eq!(result.registers.user_stack, entry.user_stack);
        assert_eq!(result.registers.status, 0x2700);
        assert_eq!(memory.roles, (0..54).collect::<Vec<_>>());
        assert_eq!(first_ccr(&memory, 0), entry.status);
        assert_eq!(first_ccr(&memory, 52), 0x2700);
        for (index, event) in memory.effects[..6].iter().enumerate() {
            let value = entry.data[4 - index / 2];
            let expected = if index & 1 == 0 {
                value as u16
            } else {
                (value >> 16) as u16
            };
            assert_eq!(event.effect.address, sp - (index as u32 + 1) * 2);
            assert_eq!(event.effect.kind, EffectKind::Write(expected));
        }
        let control: Vec<_> = memory
            .effects
            .iter()
            .filter(|event| {
                event.effect.address == VDP_CONTROL
                    && matches!(event.effect.kind, EffectKind::Write(_))
            })
            .map(|event| event.value)
            .collect();
        assert_eq!(
            control,
            [
                0x8174,
                0x9300,
                0x9400,
                0x9500 + ((source_address >> 1) & 0xff) as u16,
                0x9600 + ((source_address >> 9) & 0xff) as u16,
                0x9700 + ((source_address >> 17) & 0x7f) as u16,
                (command >> 16) as u16,
                command as u16,
            ]
        );
        let discarded: Vec<_> = memory
            .effects
            .iter()
            .filter(|event| event.effect.kind == EffectKind::Read(ReadPurpose::Discarded))
            .collect();
        assert_eq!(discarded.len(), 1);
        assert_eq!(discarded[0].effect.address, sp);
        assert_eq!(discarded[0].value, 0x0012);
        let command_address = ram::g_vdp_dma_command(revision).address();
        let publish = memory
            .effects
            .iter()
            .position(|event| {
                event.effect.address == command_address
                    && matches!(event.effect.kind, EffectKind::Write(_))
            })
            .unwrap();
        let read_high = memory
            .effects
            .iter()
            .position(|event| {
                event.effect.address == command_address
                    && matches!(event.effect.kind, EffectKind::Read(_))
            })
            .unwrap();
        assert!(publish < read_high);
    }
}

#[test]
fn user_stack_return_does_not_touch_the_inactive_supervisor_bank() {
    for leaf in [Leaf::WordStream, Leaf::Palette, Leaf::Dma] {
        let mut registers = initial_registers();
        registers.status = 0x001f;
        let entry = registers.clone();
        let mut memory = fixture(&registers);
        let sp = registers.stack_pointer();
        let source_address = 0xff9000;
        memory.put_long(sp + 4, source_address);
        memory.put_long(sp + 8, 1);
        memory.put_long(sp + 12, 1);
        memory.put_byte(ram::g_mode2_reg_softcopy(Revision::Rev02).address(), 0x60);
        for index in 0..16 {
            memory.put_word(source_address + index * 2, 0x1234);
        }
        let mut source = Source::new(Revision::Rev02, leaf);
        let result = run(&mut source, registers, &mut memory);
        assert!(matches!(
            result.path,
            Path::Return {
                address: 0x0012_3456,
                ..
            }
        ));
        assert_eq!(result.registers.user_stack, entry.user_stack + 4);
        assert_eq!(result.registers.supervisor_stack, entry.supervisor_stack);
        assert_eq!(result.registers.status & 0xffe0, 0);
    }
}

#[test]
fn protocol_rejects_missing_repeated_wrong_purpose_wrong_payload_and_wrong_sequence() {
    let revision = Revision::Rev02;
    let registers = initial_registers();
    for case in 0..7 {
        let mut source = Source::new(revision, Leaf::WordStream);
        source
            .begin_instruction(site(source.program().entry), registers.clone())
            .unwrap();
        let first = source.next_effect().unwrap().unwrap();
        match case {
            0 => assert!(source.finish_instruction().is_err()),
            1 => assert!(source.next_effect().is_err()),
            2 => {
                let wrong = Effect {
                    kind: EffectKind::Read(ReadPurpose::Discarded),
                    ..first
                };
                assert!(
                    source
                        .acknowledge(Observation {
                            effect: wrong,
                            value: 0
                        })
                        .is_err()
                );
            }
            3 => {
                source
                    .acknowledge(Observation {
                        effect: first,
                        value: 0,
                    })
                    .unwrap();
                assert!(
                    source
                        .acknowledge(Observation {
                            effect: first,
                            value: 0
                        })
                        .is_err()
                );
            }
            4 => {
                let wrong = Effect {
                    address: first.address + 2,
                    ..first
                };
                assert!(
                    source
                        .acknowledge(Observation {
                            effect: wrong,
                            value: 0
                        })
                        .is_err()
                );
            }
            5 => assert!(
                source
                    .begin_instruction(site(source.program().entry), registers.clone())
                    .is_err()
            ),
            6 => {
                source
                    .acknowledge(Observation {
                        effect: first,
                        value: 0,
                    })
                    .unwrap();
                assert!(source.finish_instruction().is_err());
            }
            _ => unreachable!(),
        }
        assert!(
            source
                .begin_instruction(site(source.program().entry), registers.clone())
                .is_err()
        );
    }
    let mut source = Source::new(revision, Leaf::Dma);
    source
        .begin_instruction(site(source.program().entry), registers)
        .unwrap();
    let effect = source.next_effect().unwrap().unwrap();
    let EffectKind::Write(value) = effect.kind else {
        panic!("MOVEM save must write")
    };
    assert!(
        source
            .acknowledge(Observation {
                effect,
                value: value ^ 1
            })
            .is_err()
    );
}

#[test]
fn shift_flags_match_independent_bit_steps_for_zero_and_large_counts() {
    for value in [
        0_u32,
        1,
        0x8000_0000,
        0xffff_ffff,
        0x7fff_ffff,
        0x4000_0000,
        0xc000_0000,
        0xa000_55aa,
    ] {
        for count in 0..64 {
            for extend in [0, 0x10] {
                for arithmetic in [false, true] {
                    let mut registers = initial_registers();
                    registers.status = 0x2700 | extend;
                    registers.data[0] = value;
                    let mut expected = value;
                    let mut carry = false;
                    let mut overflow = false;
                    for _ in 0..count {
                        let sign = expected & 0x8000_0000;
                        carry = sign != 0;
                        expected <<= 1;
                        overflow |= sign != expected & 0x8000_0000;
                    }
                    shift_left_long(&mut registers, 0, count, arithmetic);
                    let flags = if expected == 0 {
                        4
                    } else if expected & 0x8000_0000 != 0 {
                        8
                    } else {
                        0
                    };
                    let x = if count == 0 {
                        extend
                    } else if carry {
                        0x10
                    } else {
                        0
                    };
                    assert_eq!(registers.data[0], expected);
                    assert_eq!(
                        registers.status,
                        0x2700
                            | flags
                            | x
                            | u16::from(carry)
                            | if arithmetic && overflow { 2 } else { 0 }
                    );
                }
                let mut registers = initial_registers();
                registers.status = 0x2700 | extend;
                registers.data[0] = value;
                let mut expected = value;
                let mut carry = false;
                for _ in 0..count {
                    carry = expected & 1 != 0;
                    expected = ((expected as i32) >> 1) as u32;
                }
                shift_right_arithmetic(&mut registers, 0, count);
                let flags = if expected == 0 {
                    4
                } else if expected & 0x8000_0000 != 0 {
                    8
                } else {
                    0
                };
                let x = if count == 0 {
                    extend
                } else if carry {
                    0x10
                } else {
                    0
                };
                assert_eq!(registers.data[0], expected);
                assert_eq!(registers.status, 0x2700 | flags | x | u16::from(carry));
            }
        }
    }
}
