//! Authored instruction streams only; these are API/finalization regressions,
//! not proofs of the real PreGenerateRandoms translation.
use super::*;
use crate::sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use genesis_config::GenesisEmulatorConfig;
use jgenesis_common::frontend::{EmulatorTrait, PartialClone};
use tje_game::Context;

fn authored_machine(revision: Revision) -> GenesisEmulator {
    let mut rom = vec![0; 0x10000];
    rom[..4].copy_from_slice(&0x00ff7ff0_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00000200_u32.to_be_bytes());
    for offset in [0x200, 0x202, 0x204] {
        rom[offset..offset + 2].copy_from_slice(&0x4e71_u16.to_be_bytes());
    }
    // MOVE.L #...,D0; MOVE.L #...,D1; MOVE.L #...,4(SP);
    // MOVE.W #...,$FF0100; RTS. No bytes come from a user's cartridge.
    let program: [u16; 15] = [
        0x203c, 0x1357, 0x2468, 0x223c, 0x89ab, 0xcdef, 0x2f7c, 0xa1b2, 0xc3d4, 0x0004, 0x33fc,
        0x5aa5, 0x00ff, 0x0100, 0x4e75,
    ];
    let start = tje_game::ids::PRE_GENERATE_RANDOMS
        .metadata()
        .address(revision) as usize;
    for (index, opcode) in program.into_iter().enumerate() {
        rom[start + index * 2..start + index * 2 + 2].copy_from_slice(&opcode.to_be_bytes());
    }
    let mut machine = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    // Match the established synthetic-machine fixture: retire the initial
    // reset wait before seeding an ordinary, explicitly authored call frame.
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
        cpu.set_status_register(0x2700);
        set_sp(cpu, 0xff7ff0);
        cpu.set_pc(0x200, bus);
        assert_eq!(
            bus.read_word(start as u32),
            0x203c,
            "authored ROM must be mapped at the actual metadata entry"
        );
    }
    machine.hybrid_bus_mut().native_cpu_accesses = Some(Vec::new());
    machine.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    assert_eq!(machine.hybrid_cpu().pc(), 0x200);
    assert_eq!(machine.hybrid_cpu().stack_pointer(), 0xff7ff0);
    machine
}

fn run(machine: &mut GenesisEmulator, revision: Revision, prepared: bool, discard: bool) {
    let id = tje_game::ids::PRE_GENERATE_RANDOMS;
    assert!(!id.metadata().abi.returns_value);
    let mut context = HookContext::new(machine, revision, Dispatcher::hybrid(), crate::Mode::Check);
    let sp = Context::stack_pointer(&context).unwrap();
    if prepared {
        context.write_u32(sp - 4, 0x1122_3344);
        context.write_u32(sp - 8, 0x200);
        context.set_stack_pointer(sp - 8).unwrap();
        context.set_frame_state(FrameState::new(sp - 8)).unwrap();
        if discard {
            Context::call_original_entry_void(&mut context, id, ExecutionClass::Atomic).unwrap();
        } else {
            assert_eq!(
                Context::call_original_entry(&mut context, id, ExecutionClass::Atomic).unwrap(),
                0x1357_2468
            );
        }
        assert_eq!(Context::stack_pointer(&context).unwrap(), sp - 4);
    } else {
        if discard {
            Context::call_original_void_with_execution(
                &mut context,
                id,
                &[0x1122_3344],
                ExecutionClass::Atomic,
            )
            .unwrap();
        } else {
            let result = Context::call_original_with_execution(
                &mut context,
                id,
                &[0x1122_3344],
                ExecutionClass::Atomic,
            );
            let value = result.unwrap_or_else(|error| {
                panic!(
                    "{error}; authored CPU={:?}; first actual issues={:?}",
                    context.emulator.hybrid_cpu().hybrid_state_words(),
                    context
                        .emulator
                        .hybrid_bus()
                        .native_cpu_accesses
                        .as_deref()
                        .expect("enabled authored issue journal")
                        .iter()
                        .take(24)
                        .collect::<Vec<_>>()
                )
            });
            assert_eq!(value, 0x1357_2468);
        }
        assert_eq!(Context::stack_pointer(&context).unwrap(), sp);
    }
    assert_eq!(context.emulator.hybrid_cpu().pc(), 0x200);
    assert_eq!(
        context.emulator.hybrid_cpu().data_registers()[0],
        0x1357_2468
    );
    assert_eq!(
        context.emulator.hybrid_cpu().data_registers()[1],
        0x89ab_cdef
    );
    assert_eq!(context.read_u32(sp - 4), 0xa1b2_c3d4);
    assert_eq!(context.read_u16(0xff0100), 0x5aa5);
    assert_eq!(context.nested_calls, 1);
}

fn run_fork(
    origin: &mut GenesisEmulator,
    fork: &mut GenesisEmulator,
    revision: Revision,
    prepared: bool,
    discard: bool,
) {
    // PartialClone intentionally omits ROM. Match Host::execute_fork: move the
    // immutable asset for sequential oracle execution, never copy game state.
    fork.hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .take_rom_from(origin.hybrid_bus_mut().cartridge.as_mut().unwrap());
    run(fork, revision, prepared, discard);
    origin
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .take_rom_from(fork.hybrid_bus_mut().cartridge.as_mut().unwrap());
}

#[test]
fn void_original_callbacks_keep_the_exact_machine_and_frame_effects_without_a_scalar_result() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for prepared in [false, true] {
            let mut baseline = authored_machine(revision);
            let mut value = baseline.partial_clone();
            let mut discarded = baseline.partial_clone();
            run_fork(&mut baseline, &mut value, revision, prepared, false);
            run_fork(&mut baseline, &mut discarded, revision, prepared, true);
            assert_eq!(
                value.hybrid_cpu().hybrid_state_words(),
                discarded.hybrid_cpu().hybrid_state_words()
            );
            assert_eq!(
                value.hybrid_bus().memory.hybrid_raw_main_ram(),
                discarded.hybrid_bus().memory.hybrid_raw_main_ram()
            );
            assert_eq!(
                value.hybrid_bus().native_cpu_accesses,
                discarded.hybrid_bus().native_cpu_accesses
            );
            assert_eq!(
                value.hybrid_bus().hybrid_cpu_accesses,
                discarded.hybrid_bus().hybrid_cpu_accesses
            );
        }
    }
}
