use super::*;
use crate::{abi::set_sp, sinks::NoCartridgeSaves};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::native::CpuAccess;
use tje_game::execution::WaitComparison;

fn machine(width: RamWidth, flag: u32, status: u16) -> GenesisEmulator {
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&PARK_PC.to_be_bytes());
    rom[28 * 4..29 * 4].copy_from_slice(&0x300_u32.to_be_bytes());
    rom[PARK_PC as usize..PARK_PC as usize + 2].copy_from_slice(&0x60fe_u16.to_be_bytes());
    // Authored IRQ writes the flag, then returns through the real CPU frame.
    let words: Vec<u16> = match width {
        RamWidth::Byte => vec![0x13fc, 1, (flag >> 16) as u16, flag as u16, 0x4e73],
        RamWidth::Word => vec![0x33fc, 1, (flag >> 16) as u16, flag as u16, 0x4e73],
        RamWidth::Long => vec![0x23fc, 0, 1, (flag >> 16) as u16, flag as u16, 0x4e73],
    };
    for (index, word) in words.into_iter().enumerate() {
        rom[0x300 + 2 * index..0x302 + 2 * index].copy_from_slice(&word.to_be_bytes());
    }
    let mut machine = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    // Reference-only fixture advance services any power-on reset before the
    // declared test interval; no pending-IRQ or readiness flag is injected.
    machine
        .tick(
            &mut HeadlessOutput,
            &mut HeadlessAudio::default(),
            &mut Inputs(Default::default()),
            &mut NoCartridgeSaves,
        )
        .unwrap();
    {
        let (cpu, bus) = machine.hybrid_parts();
        cpu.set_status_register(status);
        set_sp(cpu, 0xff7ff0);
        cpu.set_pc(PARK_PC, bus);
    }
    let bus = machine.hybrid_bus_mut();
    bus.write_word(0xc00004, 0x8010);
    bus.write_word(0xc00004, 0x8a00);
    bus.apply_pending_writes();
    bus.native_cpu_accesses = Some(Vec::new());
    machine
}

#[test]
fn ram_predicates_poll_exact_width_through_a_real_irq_in_both_revisions() {
    for revision in [tje_rom::Revision::Rev02, tje_rom::Revision::Rev00] {
        for width in [RamWidth::Byte, RamWidth::Word, RamWidth::Long] {
            for comparison in [WaitComparison::Equal, WaitComparison::NotEqual] {
                let flag = match width {
                    RamWidth::Byte => 0xff9001,
                    RamWidth::Word => 0xff9002,
                    RamWidth::Long => 0xff9004,
                };
                let mut machine = machine(width, flag, 0x2000);
                let mut clock = WaitClock {
                    revision: Some(revision),
                    ..WaitClock::default()
                };
                let request = WaitRequest::new(WaitCondition::Ram {
                    address: flag,
                    width,
                    mask: 1,
                    value: if comparison == WaitComparison::Equal {
                        1
                    } else {
                        0
                    },
                    comparison,
                });
                let run = run_wait(&mut machine, &mut clock, request).unwrap();
                assert!(run.ticks > 0);
                assert!(
                    run.logic_trace.events.iter().any(|event| matches!(
                        event,
                        LogicEvent::ExceptionEntered { vector: 28, .. }
                    ))
                );
                assert_eq!(machine.hybrid_cpu().pc(), PARK_PC);
                assert_eq!(machine.hybrid_cpu().stack_pointer(), 0xff7ff0);
                let samples: Vec<_> = machine
                    .hybrid_bus()
                    .native_cpu_accesses
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter_map(|access| match access {
                        CpuAccess::Read {
                            address,
                            word,
                            value,
                            purpose: 0,
                        } if *address == flag
                            || (width == RamWidth::Long && *address == flag + 2) =>
                        {
                            Some((*address, *word, *value))
                        }
                        _ => None,
                    })
                    .collect();
                assert!(samples.len() >= if width == RamWidth::Long { 4 } else { 2 });
                match width {
                    RamWidth::Byte => assert!(samples.iter().all(|(_, word, _)| !word)),
                    RamWidth::Word => assert!(samples.iter().all(|(_, word, _)| *word)),
                    RamWidth::Long => {
                        assert_eq!(samples.len() % 2, 0);
                        for pair in samples.as_chunks::<2>().0 {
                            assert_eq!(
                                (pair[0].0, pair[0].1, pair[1].0, pair[1].1),
                                (flag, true, flag + 2, true)
                            );
                        }
                    }
                }
                assert_eq!(
                    samples.last().unwrap().2,
                    1,
                    "the wait owns the final satisfying read"
                );
            }
        }
    }
}

#[test]
fn already_satisfied_ram_predicate_owns_one_poll_and_long_two_word_beats() {
    for width in [RamWidth::Byte, RamWidth::Word, RamWidth::Long] {
        let flag = 0xff9000;
        let mut machine = machine(width, flag, 0x2700);
        let mut clock = WaitClock::default();
        let request = WaitRequest::new(WaitCondition::Ram {
            address: flag,
            width,
            mask: width.mask(),
            value: 0,
            comparison: WaitComparison::Equal,
        });
        let run = run_wait(&mut machine, &mut clock, request).unwrap();
        assert_eq!(run.ticks, 0);
        let samples: Vec<_> = machine.hybrid_bus().native_cpu_accesses.as_ref().unwrap().iter().filter(|access| matches!(access,
            CpuAccess::Read { address, purpose: 0, .. } if *address == flag || *address == flag + 2
        )).collect();
        assert_eq!(samples.len(), if width == RamWidth::Long { 2 } else { 1 });
    }
}

#[test]
fn impossible_or_invalid_ram_predicate_fails_instead_of_inventing_readiness() {
    let mut machine = machine(RamWidth::Byte, 0xff9001, 0x2700);
    let mut clock = WaitClock::default();
    let mut request = WaitRequest::ram_byte_equal(0xff9001, 255, 7);
    request.max_ticks = 3;
    assert!(run_wait(&mut machine, &mut clock, request).is_err());
    let mut machine = self::machine(RamWidth::Word, 0xff9002, 0x2700);
    let request = WaitRequest::ram_word_equal(0xff9001, 65535, 0);
    assert!(run_wait(&mut machine, &mut clock, request).is_err());
}

#[test]
fn poll_yield_advances_real_time_without_waiting_for_an_irq_or_reading_a_flag() {
    let mut machine = machine(RamWidth::Byte, 0xff9001, 0x2700);
    let before = machine.hybrid_bus().cycles.m68k_mclk_cycles;
    let state = machine.hybrid_cpu().hybrid_state_words();
    let run = run_poll_yield_with_audio(
        &mut machine,
        &mut WaitClock::default(),
        Vec::new(),
        true,
        PhysicalObservers::default(),
    )
    .unwrap();
    assert!(run.ticks >= 1);
    assert!(machine.hybrid_bus().cycles.m68k_mclk_cycles > before);
    assert_eq!(machine.hybrid_cpu().hybrid_state_words(), state);
    assert!(
        !machine
            .hybrid_bus()
            .native_cpu_accesses
            .as_ref()
            .unwrap()
            .iter()
            .any(|access| matches!(access,
                CpuAccess::Read { purpose: 0, address, .. } if *address >= 0xe00000
            )),
        "yield adds no software condition read"
    );
}
