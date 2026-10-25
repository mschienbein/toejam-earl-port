use super::*;
use crate::sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::native::CpuAccess;
use jgenesis_common::frontend::EmulatorTrait;
use tje_game::Platform;

fn machine() -> GenesisEmulator {
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[0x100..0x104].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfc]);
    rom[0x200..0x202].copy_from_slice(&0x60fe_u16.to_be_bytes());
    let mut machine = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
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
        cpu.set_pc(0x100, bus);
    }
    machine.hybrid_bus_mut().native_cpu_accesses = Some(Vec::new());
    machine.hybrid_bus_mut().native_ram_writes = Some(Vec::new());
    machine.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    machine
}

#[test]
fn inherent_and_trait_word_writes_issue_words_and_longs_two_ordered_words() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = machine();
        {
            let mut context = HookContext::new(
                &mut machine,
                revision,
                Dispatcher::hybrid(),
                crate::Mode::Check,
            );
            let stack = StackAuditState::new(0xff7ff0, true);
            let audit = stack.empty_run();
            context.enable_stack_audit(stack, audit);
            context.write_u16(0xff9000, 0x1234);
            tje_mem::Bus::write_u16(&mut context, 0xff9002, 0x5678);
            tje_mem::Bus::write_u32(&mut context, 0xff9004, 0xaabbccdd);
            context.write_u32(0xff9008, 0xeeff1122);
            assert_eq!(context.audit.borrow().writes.len(), 12);
        }
        let expected = [
            (0xff9000, 0x1234),
            (0xff9002, 0x5678),
            (0xff9004, 0xaabb),
            (0xff9006, 0xccdd),
            (0xff9008, 0xeeff),
            (0xff900a, 0x1122),
        ];
        let bus = machine.hybrid_bus();
        let issued = bus.native_cpu_accesses.as_ref().unwrap();
        let committed = bus.native_ram_writes.as_ref().unwrap();
        assert_eq!(issued.len(), expected.len());
        assert_eq!(committed.len(), expected.len());
        for ((issue, commit), (address, value)) in issued.iter().zip(committed).zip(expected) {
            assert_eq!(
                *issue,
                CpuAccess::Write {
                    address,
                    word: true,
                    value
                }
            );
            assert_eq!(
                (commit.address, commit.word, commit.value),
                (address, true, value)
            );
        }
    }
}

#[test]
fn inspection_word_patches_stay_external_mutations_and_keep_unaligned_storage() {
    let mut machine = machine();
    {
        let mut context = HookContext::new(
            &mut machine,
            Revision::Rev02,
            Dispatcher::hybrid(),
            crate::Mode::Check,
        )
        .with_inspection_provenance(None);
        tje_mem::Bus::write_u16(&mut context, 0xff9001, 0xabcd);
    }
    assert!(
        machine
            .hybrid_bus()
            .native_cpu_accesses
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        machine.hybrid_bus().memory.read_main_ram_byte(0xff9001),
        0xab
    );
    assert_eq!(
        machine.hybrid_bus().memory.read_main_ram_byte(0xff9002),
        0xcd
    );
}

#[test]
fn time_aware_yield_advances_devices_without_a_condition_operand() {
    let mut machine = machine();
    let before = machine.hybrid_bus().cycles.m68k_mclk_cycles;
    let state = machine.hybrid_cpu().hybrid_state_words();
    {
        let mut context = HookContext::new(
            &mut machine,
            Revision::Rev02,
            Dispatcher::hybrid(),
            crate::Mode::Check,
        )
        .with_execution(ExecutionClass::TimeAware, WaitClock::default());
        context.poll_yield().unwrap();
    }
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
            ))
    );
}

#[test]
fn atomic_yield_is_a_sticky_failure_without_time_progress() {
    let mut machine = machine();
    let before = machine.hybrid_bus().cycles.m68k_mclk_cycles;
    {
        let mut context = HookContext::new(
            &mut machine,
            Revision::Rev02,
            Dispatcher::hybrid(),
            crate::Mode::Check,
        );
        assert!(context.poll_yield().is_err());
        assert!(context.check_mmio_contract().is_err());
    }
    assert_eq!(machine.hybrid_bus().cycles.m68k_mclk_cycles, before);
}
