//! Explicit, source-backed prefetch operands for partially driven MMIO reads.
//!
//! This operation declares the original instruction's ROM prefetch; it is not an
//! ad-hoc read inserted into a game body to imitate an unrelated bus latch. The
//! real bus performs both the tagged ROM fetch and the actual device read. No
//! hardware-defined bit, readiness state, timer, clock or CPU register is forged.

use genesis_core::{GenesisBus, GenesisEmulator, HybridAccess};
use m68000_emu::{BusInterface, ReadPurpose};
use tje_game::open_bus::{OpenBusError, OpenBusSource};

type Result<T, E = OpenBusError> = std::result::Result<T, E>;

#[cfg(test)]
pub(crate) struct DeviceRead<T> {
    pub value: T,
    /// An explicit-source operand is compared in full, not with the normal
    /// defined-bit mask. No index is needed when access recording is disabled.
    pub exact_trace_index: Option<usize>,
}

/// Bits supplied by the current standalone ToeJam & Earl device decoder.
/// A predicate may observe only these without declaring a CPU prefetch source.
pub(crate) fn driven_mask(bus: &GenesisBus, address: u32, word: bool) -> u16 {
    let full = if word { 0xffff } else { 0xff };
    match address & tje_mem::ADDRESS_MASK {
        0x000000..=0x7fffff | 0xe00000..=0xffffff => full,
        0xa00000..=0xa0ffff if bus.z80_signals.busack() => full,
        0xa10000..=0xa1001f => full,
        0xa11100 => {
            if word {
                0x0100
            } else {
                0x01
            }
        }
        0xc00000..=0xc00003 | 0xc00008..=0xc0000b => full,
        0xc00004..=0xc00007 => {
            if word {
                0x03ff
            } else if address & 1 == 0 {
                0x03
            } else {
                0xff
            }
        }
        _ => 0,
    }
}

pub(crate) fn prepare(
    emulator: &mut GenesisEmulator,
    address: u32,
    word: bool,
    source: OpenBusSource,
) -> Result<&mut GenesisBus> {
    let address = address & tje_mem::ADDRESS_MASK;
    if !(0x800000..0xe00000).contains(&address) || (word && address & 1 != 0) {
        return Err(OpenBusError::InvalidDevice { address, word });
    }
    let prefetch_address = source.address()?;
    let bus = emulator.hybrid_bus_mut();
    let cartridge = bus.cartridge.as_mut().ok_or(OpenBusError::RomUnavailable)?;
    // Inspection only; never change the immutable ROM or copy its bytes.
    let rom_words = cartridge.debug_rom_view().len();
    if prefetch_address as usize / 2 >= rom_words {
        return Err(OpenBusError::MissingRomWord {
            address: prefetch_address,
        });
    }
    bus.apply_pending_writes();
    bus.read_word_tagged(prefetch_address, ReadPurpose::Prefetch);
    Ok(bus)
}

#[cfg(test)]
pub(crate) fn read_u8(
    emulator: &mut GenesisEmulator,
    address: u32,
    source: OpenBusSource,
) -> Result<DeviceRead<u8>> {
    let bus = prepare(emulator, address, false, source)?;
    let exact_trace_index = bus.hybrid_accesses.as_ref().map(Vec::len);
    let value = bus.read_byte(address);
    Ok(DeviceRead {
        value,
        exact_trace_index,
    })
}

#[cfg(test)]
pub(crate) fn read_u16(
    emulator: &mut GenesisEmulator,
    address: u32,
    source: OpenBusSource,
) -> Result<DeviceRead<u16>> {
    let bus = prepare(emulator, address, true, source)?;
    let exact_trace_index = bus.hybrid_accesses.as_ref().map(Vec::len);
    let value = bus.read_word(address);
    Ok(DeviceRead {
        value,
        exact_trace_index,
    })
}

/// Additional obligation for explicit-source reads. The ordinary transaction
/// comparison still checks every event; this adds full operand equality at the
/// marked positions, including bits normally treated as undefined/open bus.
/// Read purpose remains an audit classification, not a hardware value bit.
pub(crate) fn compare_exact_reads(
    original: &[HybridAccess],
    translated: &[HybridAccess],
    exact_indices: &[usize],
) -> std::result::Result<(), String> {
    for &index in exact_indices {
        match (original.get(index), translated.get(index)) {
            (
                Some(HybridAccess::Read {
                    address: aa,
                    word: aw,
                    value: av,
                    ..
                }),
                Some(HybridAccess::Read {
                    address: ba,
                    word: bw,
                    value: bv,
                    ..
                }),
            ) if aa == ba && aw == bw && av == bv => {}
            (a, b) => {
                return Err(format!(
                    "explicit open-bus operand differs at trace {index}: original={a:?}, Rust={b:?}"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sinks::NoCartridgeSaves;
    use genesis_config::GenesisEmulatorConfig;

    fn emulator() -> GenesisEmulator {
        // Synthetic ROM/vector data, not game bytes. The two source words are
        // deliberately different so an unrelated prefetch cannot pass unnoticed.
        let mut rom = vec![0; 1024];
        rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
        rom[0x100..0x104].copy_from_slice(&[0x12, 0x34, 0xab, 0xcd]);
        // Authored JSR supplies the source-backed return PC for frame tests.
        rom[0x200..0x202].copy_from_slice(&0x4eb9_u16.to_be_bytes());
        rom[0x202..0x206].copy_from_slice(
            &tje_game::ids::GET_RANDOM_NUMBER
                .metadata()
                .address(tje_rom::Revision::Rev02)
                .to_be_bytes(),
        );
        let mut emulator = GenesisEmulator::create(
            Some(rom),
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        emulator.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
        emulator
    }

    fn write(emulator: &mut GenesisEmulator, address: u32, value: u16) {
        let bus = emulator.hybrid_bus_mut();
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }

    #[test]
    fn busack_retains_real_grant_and_actual_prefetch_bits() {
        let mut emulator = emulator();
        write(&mut emulator, 0xa11100, 0x100);
        let denied = read_u16(&mut emulator, 0xa11100, OpenBusSource::prefetch(0x100)).unwrap();
        assert_eq!(denied.value, 0x1334);
        write(&mut emulator, 0xa11200, 0x100);
        let granted = read_u16(&mut emulator, 0xa11100, OpenBusSource::prefetch(0x102)).unwrap();
        assert_eq!(granted.value, 0xaacd);
        assert_eq!(
            read_u8(&mut emulator, 0xa11100, OpenBusSource::prefetch(0x102))
                .unwrap()
                .value,
            0xaa
        );
        assert_eq!(
            read_u8(&mut emulator, 0xa11101, OpenBusSource::prefetch(0x102))
                .unwrap()
                .value,
            0xcd
        );
        let index = granted.exact_trace_index.unwrap();
        assert!(matches!(
            emulator
                .hybrid_bus()
                .hybrid_accesses
                .as_ref()
                .unwrap()
                .get(index),
            Some(HybridAccess::Read {
                address: 0xa11100,
                word: true,
                value: 0xaacd,
                ..
            })
        ));
    }

    #[test]
    fn vdp_status_keeps_device_bits_and_real_latch_side_effects() {
        let mut emulator = emulator();
        let status = emulator.hybrid_bus_mut().read_word(0xc00004) & 0x03ff;
        write(&mut emulator, 0xc00004, 0x4000);
        let value = read_u16(&mut emulator, 0xc00004, OpenBusSource::prefetch(0x102))
            .unwrap()
            .value;
        assert_eq!(value & 0xfc00, 0xa800);
        assert_eq!(value & 0x03ff, status);
        write(&mut emulator, 0xc00004, 0x8f02);
        assert_eq!(
            emulator
                .hybrid_bus()
                .vdp
                .hybrid_registers()
                .data_port_auto_increment,
            2
        );
        emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
        read_u16(&mut emulator, 0xc00004, OpenBusSource::prefetch(0x100)).unwrap();
        assert_eq!(
            emulator.hybrid_bus().hybrid_functional_violation,
            Some(0xc00004)
        );
    }

    #[test]
    fn controller_pins_are_not_replaced_by_cpu_prefetch_bits() {
        let mut emulator = emulator();
        let byte = emulator.hybrid_bus_mut().read_byte(0xa10003);
        let first = read_u16(&mut emulator, 0xa10003 & !1, OpenBusSource::prefetch(0x100))
            .unwrap()
            .value;
        let second = read_u16(&mut emulator, 0xa10003 & !1, OpenBusSource::prefetch(0x102))
            .unwrap()
            .value;
        assert_eq!(first, u16::from_be_bytes([byte; 2]));
        assert_eq!(second, first);
    }

    #[test]
    fn write_only_holes_and_denied_z80_window_use_the_same_rom_source() {
        let mut emulator = emulator();
        let source = OpenBusSource::prefetch(0x102);
        assert_eq!(
            read_u16(&mut emulator, 0xc00010, source).unwrap().value,
            0xabcd
        );
        assert_eq!(
            read_u8(&mut emulator, 0xc00011, source).unwrap().value,
            0xcd
        );
        assert_eq!(
            read_u16(&mut emulator, 0xa11200, source).unwrap().value,
            0xabcd
        );
        assert_eq!(
            read_u16(&mut emulator, 0xa00000, source).unwrap().value,
            0xabcd
        );
        write(&mut emulator, 0xa11100, 0x100);
        write(&mut emulator, 0xa11200, 0x100);
        emulator.hybrid_bus_mut().write_byte(0xa00000, 0x5a);
        emulator.hybrid_bus_mut().apply_pending_writes();
        assert_eq!(
            read_u16(&mut emulator, 0xa00000, source).unwrap().value,
            0x5a5a
        );
    }

    #[test]
    fn invalid_sources_and_word_alignment_fail_before_any_device_read() {
        let mut emulator = emulator();
        for source in [1, 1024, 0xff8000, 0xa04000] {
            assert!(read_u16(&mut emulator, 0xa11100, OpenBusSource::prefetch(source)).is_err());
        }
        assert!(read_u16(&mut emulator, 0xa11101, OpenBusSource::prefetch(0x100)).is_err());
        assert!(read_u8(&mut emulator, 0xff8000, OpenBusSource::prefetch(0x100)).is_err());
        assert!(
            emulator
                .hybrid_bus()
                .hybrid_accesses
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn explicit_reads_compare_full_operands_even_if_defined_bits_match() {
        let read = |value| HybridAccess::Read {
            address: 0xa11100,
            word: true,
            value,
            purpose: ReadPurpose::Semantic,
        };
        let original = [read(0x1234)];
        let changed = [read(0x9234)];
        assert!(crate::accesses_match(&original[0], &changed[0]));
        assert!(compare_exact_reads(&original, &changed, &[0]).is_err());
        assert!(compare_exact_reads(&original, &original, &[0]).is_ok());
        assert!(compare_exact_reads(&original, &original, &[1]).is_err());
    }
    fn invalid_reader(
        context: &mut dyn tje_game::Context,
        _: &[u32],
    ) -> tje_game::Result<Option<u32>> {
        let count = context.read_u8(0xff9000);
        context.write_u8(0xff9000, count.wrapping_add(1));
        let _ = context.read_device_u16(0xa11100);
        Ok(None)
    }

    static INVALID_READER: &[tje_game::Translation] = &[tje_game::Translation {
        id: tje_game::ids::GET_RANDOM_NUMBER,
        function: invalid_reader,
    }];

    #[test]
    fn driven_masks_cover_partial_and_denied_device_reads() {
        let mut emulator = emulator();
        let bus = emulator.hybrid_bus();
        for (address, word, mask) in [
            (0xa11100, true, 0x0100),
            (0xa11100, false, 1),
            (0xa11101, false, 0),
            (0xc00004, true, 0x03ff),
            (0xc00004, false, 3),
            (0xc00005, false, 0xff),
            (0xc00000, true, 0xffff),
            (0xc00008, true, 0xffff),
            (0xa10002, true, 0xffff),
            (0xc00010, true, 0),
            (0xa00000, true, 0),
            (0xff8000, true, 0xffff),
        ] {
            assert_eq!(driven_mask(bus, address, word), mask);
        }
        write(&mut emulator, 0xa11100, 0x100);
        write(&mut emulator, 0xa11200, 0x100);
        assert_eq!(driven_mask(emulator.hybrid_bus(), 0xa04000, false), 0xff);
    }

    #[test]
    fn discarded_annotation_cannot_hide_a_semantic_operand() {
        let read = |purpose| HybridAccess::Read {
            address: 0xa11100,
            word: true,
            value: 0x1234,
            purpose,
        };
        assert!(!crate::accesses_match(
            &read(ReadPurpose::Semantic),
            &read(ReadPurpose::Discarded)
        ));
        assert!(crate::accesses_match(
            &read(ReadPurpose::Discarded),
            &read(ReadPurpose::Discarded)
        ));
    }

    #[test]
    fn missing_source_access_is_real_and_poison_survives_nested_calls() {
        use tje_game::{Context, Platform};
        let mut emulator = emulator();
        let dispatcher = tje_game::Dispatcher::hybrid_with(INVALID_READER);
        {
            let mut context = crate::abi::HookContext::new(
                &mut emulator,
                tje_rom::Revision::Rev02,
                dispatcher,
                crate::Mode::Replace,
            )
            .with_execution(
                tje_game::execution::ExecutionClass::TimeAware,
                crate::execution::WaitClock::default(),
            );
            let id = tje_game::ids::GET_RANDOM_NUMBER;
            assert!(matches!(
                dispatcher.call(&mut context, id, &[]),
                Err(tje_game::Error::OpenBus(OpenBusError::SourceRequired {
                    address: 0xa11100,
                    word: true
                }))
            ));
            assert_eq!(
                context.execution_class(),
                tje_game::execution::ExecutionClass::TimeAware
            );
            assert!(dispatcher.call(&mut context, id, &[]).is_err());
            assert_eq!(context.read_u8(0xff9000), 1);
            assert!(context.return_to_caller(None).is_err());
            assert!(context.check_mmio_contract().is_err());
        }
        assert!(matches!(
            emulator
                .hybrid_bus()
                .hybrid_accesses
                .as_ref()
                .unwrap()
                .as_slice(),
            [HybridAccess::Read {
                address: 0xa11100,
                ..
            }]
        ));
    }

    #[test]
    fn framed_call_cleans_up_but_cannot_return_success_after_violation() {
        use tje_game::Context;
        let mut emulator = emulator();
        let dispatcher = tje_game::Dispatcher::hybrid_with(INVALID_READER);
        let mut context = crate::abi::HookContext::new(
            &mut emulator,
            tje_rom::Revision::Rev02,
            dispatcher,
            crate::Mode::Replace,
        );
        let before = context.stack_pointer().unwrap();
        let site = tje_game::call_frame::CallSite::new(0x206);
        assert!(
            dispatcher
                .call_at(&mut context, tje_game::ids::GET_RANDOM_NUMBER, &[], site)
                .is_err()
        );
        assert_eq!(context.stack_pointer().unwrap(), before);
    }

    #[test]
    fn caught_invalid_source_remains_fatal_before_original_bridge() {
        use tje_game::Platform;
        let mut emulator = emulator();
        let dispatcher = tje_game::Dispatcher::hybrid();
        let mut context = crate::abi::HookContext::new(
            &mut emulator,
            tje_rom::Revision::Rev02,
            dispatcher,
            crate::Mode::Replace,
        );
        let before = (context.cpu().pc(), context.cpu().stack_pointer());
        assert!(
            context
                .read_device_u16_with_open_bus(0xa11100, OpenBusSource::prefetch(1))
                .is_err()
        );
        let _ = context.read_device_u8(0xa10003);
        assert!(matches!(
            context.check_mmio_contract(),
            Err(tje_game::Error::OpenBus(OpenBusError::InvalidSource {
                address: 1
            }))
        ));
        assert!(
            context
                .call_original_with_execution_contract(
                    tje_game::ids::GET_RANDOM_NUMBER.metadata(),
                    tje_game::execution::ExecutionClass::Atomic,
                    &[]
                )
                .is_err()
        );
        assert_eq!((context.cpu().pc(), context.cpu().stack_pointer()), before);
    }

    #[test]
    fn wait_predicate_cannot_observe_undefined_bits_from_the_idle_loop() {
        use tje_game::Platform;
        // This case executes a wait, unlike the JSR-based frame fixtures above.
        let mut rom = vec![0; 1024];
        rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
        rom[0x200..0x202].copy_from_slice(&0x60fe_u16.to_be_bytes());
        let mut emulator = GenesisEmulator::create(
            Some(rom),
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let mut context = crate::abi::HookContext::new(
            &mut emulator,
            tje_rom::Revision::Rev02,
            tje_game::Dispatcher::hybrid(),
            crate::Mode::Replace,
        )
        .with_execution(
            tje_game::execution::ExecutionClass::TimeAware,
            crate::execution::WaitClock::default(),
        );
        let request =
            tje_game::execution::WaitRequest::new(tje_game::execution::WaitCondition::DeviceWord {
                address: 0xa11100,
                mask: 0xffff,
                value: 0,
            });
        assert!(matches!(
            context.wait_until(request),
            Err(tje_game::Error::OpenBus(
                OpenBusError::SourceRequired { .. }
            ))
        ));
        assert!(context.check_mmio_contract().is_err());
    }

    #[test]
    fn source_and_discarded_reads_share_the_dependent_bus_observer() {
        use tje_game::Platform;
        let mut emulator = emulator();
        emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        let exact_index;
        {
            let mut context = crate::abi::HookContext::new(
                &mut emulator,
                tje_rom::Revision::Rev02,
                tje_game::Dispatcher::hybrid(),
                crate::Mode::Replace,
            );
            let stack = crate::abi::StackAuditState::new(context.cpu().stack_pointer(), true);
            let audit = stack.empty_run();
            context.enable_stack_audit(stack, audit);
            context.read_device_u16(0xff7fec);
            assert_eq!(
                context
                    .read_device_u16_with_open_bus(0xa11100, OpenBusSource::prefetch(0x100))
                    .unwrap(),
                0x1334,
            );
            context.discard_u16(0xa11100).unwrap();
            assert!(context.check_mmio_contract().is_ok());
            assert_eq!(
                context.audit.borrow().stack_reads.as_ref().unwrap().len(),
                1
            );
            assert_eq!(context.exact_device_reads.len(), 1);
            exact_index = context.exact_device_reads[0];
        }
        let accesses = emulator.hybrid_bus().hybrid_accesses.as_ref().unwrap();
        assert_eq!(accesses.len(), exact_index + 2);
        assert!(matches!(
            accesses[exact_index],
            HybridAccess::Read {
                address: 0xa11100,
                word: true,
                value: 0x1334,
                purpose: ReadPurpose::Semantic,
            }
        ));
        assert!(matches!(
            accesses[exact_index + 1],
            HybridAccess::Read {
                address: 0xa11100,
                word: true,
                purpose: ReadPurpose::Discarded,
                ..
            }
        ));
    }

    #[test]
    fn prepared_tagged_reader_composes_with_actual_dma_commit_audit() {
        use tje_game::Platform;
        let mut emulator = emulator();
        emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
        let exact_index;
        {
            let mut context = crate::abi::HookContext::new(
                &mut emulator,
                tje_rom::Revision::Rev02,
                tje_game::Dispatcher::hybrid(),
                crate::Mode::Replace,
            );
            let stack = crate::abi::StackAuditState::new(context.cpu().stack_pointer(), true);
            let audit = stack.empty_run();
            context.enable_stack_audit(stack, audit);
            context.read_device_u16(0xff7fec);
            context.write_u16(0xff7fec, 0xabcd);
            // Authored one-word RAM-to-VRAM transaction through the same bus.
            for value in [
                0x8114, 0x8f02, 0x9301, 0x9400, 0x95f6, 0x96bf, 0x977f, 0x4200, 0x0080,
            ] {
                context.write_device_u16(0xc00004, value);
            }
            assert_eq!(
                context
                    .read_device_u16_with_open_bus(0xa11100, OpenBusSource::prefetch(0x100))
                    .unwrap(),
                0x1334,
            );
            context.discard_u16(0xa11100).unwrap();
            assert!(context.check_mmio_contract().is_ok());
            let audit = context.audit.borrow();
            let reads = audit.stack_reads.as_ref().unwrap();
            assert_eq!(reads.len(), 2);
            assert_eq!(reads[0].read.origin, "cpu");
            assert_eq!(reads[0].uninitialized_mask, 3);
            assert_eq!(reads[1].read.origin, "vdp_dma");
            assert_eq!(reads[1].read.raw_address, 0xff7fec);
            assert_eq!(reads[1].read.value, 0xabcd);
            assert_eq!(reads[1].uninitialized_mask, 0);
            assert_eq!(audit.uninitialized_reads.len(), 2);
            assert_eq!(context.exact_device_reads.len(), 1);
            exact_index = context.exact_device_reads[0];
        }
        let accesses = emulator.hybrid_bus().hybrid_accesses.as_ref().unwrap();
        assert_eq!(accesses.len(), exact_index + 2);
        assert!(matches!(
            accesses[exact_index],
            HybridAccess::Read {
                address: 0xa11100,
                purpose: ReadPurpose::Semantic,
                value: 0x1334,
                ..
            }
        ));
        assert!(matches!(
            accesses[exact_index + 1],
            HybridAccess::Read {
                address: 0xa11100,
                purpose: ReadPurpose::Discarded,
                ..
            }
        ));
    }

    fn invalid_framed_reader(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
        invalid_reader(context, &[])
    }

    static INVALID_FRAMED_READER: &[tje_game::FramedTranslation] = &[tje_game::FramedTranslation {
        id: tje_game::ids::GET_RANDOM_NUMBER,
        function: invalid_framed_reader,
    }];

    #[test]
    fn lazy_framed_and_typed_entry_paths_validate_before_success() {
        use tje_game::Context;
        for typed_entry in [false, true] {
            let mut emulator = emulator();
            let dispatcher =
                tje_game::Dispatcher::hybrid_with_overrides_and_frames(&[], INVALID_FRAMED_READER);
            let mut context = crate::abi::HookContext::new(
                &mut emulator,
                tje_rom::Revision::Rev02,
                dispatcher,
                crate::Mode::Replace,
            )
            .with_execution(
                tje_game::execution::ExecutionClass::TimeAware,
                crate::execution::WaitClock::default(),
            );
            let before = context.stack_pointer().unwrap();
            let result = if typed_entry {
                tje_game::exits::enter(&mut context, tje_game::ids::GET_RANDOM_NUMBER, &[])
                    .map(|_| 0)
            } else {
                dispatcher.call_at(
                    &mut context,
                    tje_game::ids::GET_RANDOM_NUMBER,
                    &[],
                    tje_game::call_frame::CallSite::new(0x206),
                )
            };
            assert!(matches!(
                result,
                Err(tje_game::Error::OpenBus(
                    OpenBusError::SourceRequired { .. }
                ))
            ));
            assert_eq!(
                context.execution_class(),
                tje_game::execution::ExecutionClass::TimeAware
            );
            assert_eq!(context.stack_pointer().unwrap(), before);
        }
    }

    #[test]
    fn read_only_z80_signal_receipts_are_actual_fields_without_mmio() {
        let mut emulator = emulator();
        let signals = emulator.hybrid_bus().z80_signals;
        assert!(!signals.bus_requested());
        assert!(signals.reset_asserted());
        assert!(!signals.busack());
        write(&mut emulator, 0xa11100, 0x100);
        write(&mut emulator, 0xa11200, 0x100);
        emulator
            .hybrid_bus_mut()
            .hybrid_accesses
            .as_mut()
            .unwrap()
            .clear();
        let signals = emulator.hybrid_bus().z80_signals;
        assert!(signals.bus_requested());
        assert!(!signals.reset_asserted());
        assert!(signals.busack());
        assert!(
            emulator
                .hybrid_bus()
                .hybrid_accesses
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }
}
