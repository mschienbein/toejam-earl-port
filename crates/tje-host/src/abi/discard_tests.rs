use super::*;
use crate::sinks::NoCartridgeSaves;
use genesis_config::GenesisEmulatorConfig;
use genesis_core::native::CpuAccess;
use tje_game::Platform;

fn machine() -> GenesisEmulator {
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
    rom[0x100..0x104].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    rom[0x200..0x202].copy_from_slice(&0x60fe_u16.to_be_bytes());
    let mut machine = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    machine.hybrid_parts().0.set_status_register(0x2700);
    set_sp(machine.hybrid_parts().0, 0xff7ff0);
    machine
        .hybrid_bus_mut()
        .memory
        .write_main_ram::<true>(0xff7fec, 0xabcd);
    machine.hybrid_bus_mut().native_cpu_accesses = Some(Vec::new());
    machine.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    machine
}

#[test]
fn generic_discard_is_one_actual_width_tagged_read_without_a_software_operand() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = machine();
        let before = machine.hybrid_cpu().hybrid_state_words();
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
            let (): () = context.discard_u8(0x100).unwrap();
            let (): () = context.discard_u16(0x100).unwrap();
            let (): () = context.discard_u8(0xff7fec).unwrap();
            let (): () = context.discard_u16(0xff7fec).unwrap();
            let (): () = context.discard_u16(0xa11100).unwrap();
            assert!(context.check_mmio_contract().is_ok());
            assert!(context.audit.borrow().uninitialized_reads.is_empty());
            assert_eq!(context.read_u16(0xff7fec), 0xabcd);
            assert_eq!(context.audit.borrow().uninitialized_reads.len(), 2);
        }
        assert_eq!(machine.hybrid_cpu().hybrid_state_words(), before);
        let reads = machine.hybrid_bus().native_cpu_accesses.as_ref().unwrap();
        assert_eq!(
            reads.len(),
            5,
            "no semantic pre-read, duplicate, or byte-split word"
        );
        for (access, (address, word, expected)) in reads.iter().zip([
            (0x100, false, Some(0x12)),
            (0x100, true, Some(0x1234)),
            (0xff7fec, false, Some(0xab)),
            (0xff7fec, true, Some(0xabcd)),
            (0xa11100, true, None),
        ]) {
            match access {
                CpuAccess::Read {
                    address: actual,
                    word: width,
                    value,
                    purpose: 2,
                } => {
                    assert_eq!((*actual, *width), (address, word));
                    if let Some(expected) = expected {
                        assert_eq!(*value, expected);
                    }
                }
                other => panic!("not the single actual discarded read: {other:?}"),
            }
        }
    }
}

#[test]
fn discarded_status_read_keeps_the_real_vdp_control_latch_effect() {
    let mut machine = machine();
    {
        let mut context = HookContext::new(
            &mut machine,
            Revision::Rev02,
            Dispatcher::hybrid(),
            crate::Mode::Check,
        );
        context.write_device_u16(0xc00004, 0x4000);
    }
    assert!(
        machine
            .hybrid_bus()
            .vdp
            .hybrid_checkpoint()
            .control_word_pending
    );
    {
        let mut context = HookContext::new(
            &mut machine,
            Revision::Rev02,
            Dispatcher::hybrid(),
            crate::Mode::Check,
        );
        context.discard_u16(0xc00004).unwrap();
    }
    assert!(
        !machine
            .hybrid_bus()
            .vdp
            .hybrid_checkpoint()
            .control_word_pending
    );
    let reads = machine.hybrid_bus().native_cpu_accesses.as_ref().unwrap();
    assert_eq!(
        reads
            .iter()
            .filter(|access| matches!(
                access,
                CpuAccess::Read {
                    address: 0xc00004,
                    word: true,
                    purpose: 2,
                    ..
                }
            ))
            .count(),
        1
    );
}
