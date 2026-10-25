//! Authored cartridge assets only. Pointer checks concern ROM ownership, never
//! function/body/proof admission or comparison by pointer membership.
use super::*;
use m68000_emu::BusInterface;

fn machine() -> GenesisEmulator {
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[0x100..0x104].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfe]);
    rom[0x300..0x304].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    )
}
fn rom_pointer(machine: &mut GenesisEmulator) -> *const u16 {
    machine
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view()
        .as_ptr()
}
fn rom_words(machine: &mut GenesisEmulator) -> Vec<u16> {
    machine
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view()
        .to_vec()
}
fn mutate_fork(fork: &mut GenesisEmulator) {
    let (cpu, bus) = fork.hybrid_parts();
    let mut data = cpu.data_registers();
    data[0] = 0x1234_5678;
    cpu.set_data_registers(data);
    bus.memory.write_main_ram_byte(0xff9000, 0x5a);
    bus.write_word(0xc00004, 0x8f1d);
    bus.apply_pending_writes();
}
fn vdp(machine: &GenesisEmulator) -> Vec<u8> {
    bincode::encode_to_vec(&machine.hybrid_bus().vdp, bincode::config::standard()).unwrap()
}
fn assert_returned(
    origin: &mut GenesisEmulator,
    fork: &mut GenesisEmulator,
    pointer: *const u16,
    words: &[u16],
    cpu: &[u32; 25],
    ram: &[u16],
    vdp_before: &[u8],
) {
    assert_eq!(
        rom_pointer(origin),
        pointer,
        "same allocation returned, not a ROM copy"
    );
    assert_eq!(rom_words(origin), words);
    assert!(rom_words(fork).is_empty(), "fork no longer owns the asset");
    assert_eq!(origin.hybrid_cpu().hybrid_state_words(), *cpu);
    assert_eq!(origin.hybrid_bus().memory.hybrid_raw_main_ram(), ram);
    assert_eq!(vdp(origin), vdp_before);
    assert_eq!(fork.hybrid_cpu().data_registers()[0], 0x1234_5678);
    assert_eq!(fork.hybrid_bus().memory.read_main_ram_byte(0xff9000), 0x5a);
    assert_ne!(
        vdp(fork),
        vdp_before,
        "callback effects stay with fork; no snapshot repair"
    );
}

#[test]
fn success_and_result_error_return_exact_rom_without_restoring_game_snapshots() {
    for fails in [false, true] {
        let mut origin = machine();
        let pointer = rom_pointer(&mut origin);
        let words = rom_words(&mut origin);
        let cpu = origin.hybrid_cpu().hybrid_state_words();
        let ram = origin.hybrid_bus().memory.hybrid_raw_main_ram().to_vec();
        let vdp_before = vdp(&origin);
        let mut fork = origin.partial_clone();
        assert!(rom_words(&mut fork).is_empty());
        let result = Host::execute_rom_fork(&mut origin, &mut fork, |fork| {
            assert_eq!(rom_pointer(fork), pointer);
            assert_eq!(
                fork.hybrid_bus()
                    .cartridge
                    .as_ref()
                    .unwrap()
                    .peek_word(0x300),
                0x1234
            );
            mutate_fork(fork);
            if fails {
                Err(HostError::Metadata("authored closure error".into()))
            } else {
                Ok(42_u32)
            }
        });
        if fails {
            assert!(
                matches!(result, Err(HostError::Metadata(detail)) if detail == "authored closure error")
            );
        } else {
            assert_eq!(result.unwrap(), 42);
        }
        assert_returned(
            &mut origin,
            &mut fork,
            pointer,
            &words,
            &cpu,
            &ram,
            &vdp_before,
        );
    }
}

#[derive(Debug)]
struct OriginalPayload {
    marker: u64,
}
#[test]
fn unwind_restores_exact_rom_allocation_and_resumes_the_same_owned_payload() {
    let mut origin = machine();
    let pointer = rom_pointer(&mut origin);
    let words = rom_words(&mut origin);
    let cpu = origin.hybrid_cpu().hybrid_state_words();
    let ram = origin.hybrid_bus().memory.hybrid_raw_main_ram().to_vec();
    let vdp_before = vdp(&origin);
    let mut fork = origin.partial_clone();
    let payload = Box::new(OriginalPayload {
        marker: 0x1122_3344_5566_7788,
    });
    let payload_pointer: *const OriginalPayload = &*payload;
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = Host::execute_rom_fork(&mut origin, &mut fork, |fork| {
            assert_eq!(rom_pointer(fork), pointer);
            mutate_fork(fork);
            std::panic::resume_unwind(payload)
        });
    }));
    let payload = caught
        .expect_err("original unwind must not become a Result or success")
        .downcast::<OriginalPayload>()
        .expect("original payload type retained");
    assert!(
        std::ptr::eq(&*payload, payload_pointer),
        "same owned unwind payload, not conversion/clone"
    );
    assert_eq!(payload.marker, 0x1122_3344_5566_7788);
    assert_returned(
        &mut origin,
        &mut fork,
        pointer,
        &words,
        &cpu,
        &ram,
        &vdp_before,
    );
}
