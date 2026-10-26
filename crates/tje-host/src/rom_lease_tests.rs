//! Authored assets only. Allocation checks concern analysis-ROM ownership, never
//! function/body/proof admission or comparison by pointer membership.
use super::*;
use m68000_emu::BusInterface;

fn rom() -> Vec<u8> {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    bytes[0x100..0x104].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfe]);
    bytes[0x300..0x304].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    bytes
}
fn host() -> Host {
    let bytes = rom();
    let config = GenesisEmulatorConfig::default();
    let mut host = fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
    host.analysis_rom = Some(analysis_rom::AnalysisRom::new(&bytes, &config));
    host.emulator = GenesisEmulator::create(Some(bytes), config, &mut NoCartridgeSaves);
    host
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
fn encoded(machine: &GenesisEmulator) -> Vec<u8> {
    bincode::encode_to_vec(machine, bincode::config::standard()).unwrap()
}
fn asset(host: &mut Host) -> (*const u16, Vec<u16>) {
    let mut fork = host.fork();
    host.execute_fork(&mut fork, |fork| Ok((rom_pointer(fork), rom_words(fork))))
        .unwrap()
}
fn assert_returned(
    host: &mut Host,
    fork: &mut GenesisEmulator,
    allocation: (*const u16, Vec<u16>),
    before: &[u8],
) {
    assert_eq!(
        asset(host),
        allocation,
        "same analysis allocation and bytes returned"
    );
    assert!(rom_words(fork).is_empty(), "fork no longer owns the asset");
    assert_eq!(
        encoded(&host.emulator),
        before,
        "live state is never repaired"
    );
    assert_eq!(
        host.emulator
            .hybrid_bus()
            .cartridge
            .as_ref()
            .unwrap()
            .rom_len(),
        1024
    );
    assert_eq!(
        host.emulator
            .hybrid_bus()
            .cartridge
            .as_ref()
            .unwrap()
            .peek_word(0x300),
        0x1234
    );
    assert_eq!(fork.hybrid_cpu().data_registers()[0], 0x1234_5678);
    assert_eq!(fork.hybrid_bus().memory.read_main_ram_byte(0xff9000), 0x5a);
    assert_ne!(
        fork.hybrid_bus()
            .vdp
            .hybrid_registers()
            .data_port_auto_increment,
        host.emulator
            .hybrid_bus()
            .vdp
            .hybrid_registers()
            .data_port_auto_increment,
        "callback effects stay with fork; no snapshot repair"
    );
}

#[test]
fn success_and_result_error_return_exact_analysis_asset_not_live_rom() {
    for fails in [false, true] {
        let mut host = host();
        let allocation = asset(&mut host);
        let before = encoded(&host.emulator);
        let mut fork = host.fork();
        assert!(rom_words(&mut fork).is_empty());
        let result = host.execute_fork(&mut fork, |fork| {
            assert_eq!(rom_pointer(fork), allocation.0);
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
        assert_returned(&mut host, &mut fork, allocation, &before);
    }
}

#[derive(Debug)]
struct OriginalPayload {
    marker: u64,
}
#[test]
fn unwind_returns_exact_analysis_allocation_and_resumes_same_owned_payload() {
    let mut host = host();
    let allocation = asset(&mut host);
    let before = encoded(&host.emulator);
    let mut fork = host.fork();
    let payload = Box::new(OriginalPayload {
        marker: 0x1122_3344_5566_7788,
    });
    let pointer: *const OriginalPayload = &*payload;
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = host.execute_fork(&mut fork, |fork| {
            assert_eq!(rom_pointer(fork), allocation.0);
            mutate_fork(fork);
            std::panic::resume_unwind(payload)
        });
    }));
    let payload = caught
        .expect_err("original unwind must not become a Result or success")
        .downcast::<OriginalPayload>()
        .expect("original payload type retained");
    assert!(
        std::ptr::eq(&*payload, pointer),
        "same owned unwind payload"
    );
    assert_eq!(payload.marker, 0x1122_3344_5566_7788);
    assert_returned(&mut host, &mut fork, allocation, &before);
}

#[test]
fn repeated_loans_allocate_nothing_and_never_copy_the_cached_asset() {
    let mut host = host();
    let pointer = asset(&mut host).0;
    let mut fork = host.fork();
    allocation_counter::ALLOCATIONS.with(|counter| counter.set((true, 0, 0)));
    for _ in 0..8 {
        host.execute_fork(&mut fork, |fork| {
            assert_eq!(rom_pointer(fork), pointer);
            Ok(())
        })
        .unwrap();
    }
    let (_, calls, bytes) =
        allocation_counter::ALLOCATIONS.with(|counter| counter.replace((false, 0, 0)));
    assert_eq!(
        (calls, bytes),
        (0, 0),
        "no ROM-sized copy or other loan allocation"
    );
}

#[test]
fn private_analysis_host_and_its_nested_forks_share_one_asset_on_all_exits() {
    for outcome in 0..3 {
        let mut host = host();
        let allocation = asset(&mut host);
        let before = encoded(&host.emulator);
        let mut analysis = fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
        analysis.emulator = host.fork();
        analysis.analysis_rom = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.with_fork_rom(&mut analysis, |analysis| {
                assert_eq!(rom_pointer(&mut analysis.emulator), allocation.0);
                let mut child = analysis.fork();
                analysis.execute_fork(&mut child, |child| {
                    assert_eq!(rom_pointer(child), allocation.0);
                    mutate_fork(child);
                    Ok(())
                })?;
                assert!(rom_words(&mut child).is_empty());
                assert_eq!(rom_pointer(&mut analysis.emulator), allocation.0);
                mutate_fork(&mut analysis.emulator);
                match outcome {
                    0 => Ok(()),
                    1 => Err(HostError::Metadata("nested authored error".into())),
                    _ => std::panic::resume_unwind(Box::new(OriginalPayload { marker: 7 })),
                }
            })
        }));
        match outcome {
            0 => result.unwrap().unwrap(),
            1 => assert!(matches!(result.unwrap(), Err(HostError::Metadata(_)))),
            _ => assert_eq!(
                result
                    .unwrap_err()
                    .downcast::<OriginalPayload>()
                    .unwrap()
                    .marker,
                7
            ),
        }
        assert!(
            analysis.analysis_rom.is_none(),
            "loan marker is not reusable authority"
        );
        assert_returned(&mut host, &mut analysis.emulator, allocation, &before);
    }
}

#[test]
fn missing_depot_and_nonempty_destinations_fail_without_replacing_assets() {
    let mut host = host();
    let allocation = asset(&mut host);
    let mut occupied = self::host();
    let occupied_before = encoded(&occupied.emulator);
    assert!(
        host.execute_fork::<()>(&mut occupied.emulator, |_| panic!("occupied target"))
            .is_err()
    );
    assert_eq!(encoded(&occupied.emulator), occupied_before);
    assert_eq!(asset(&mut host), allocation);
    assert!(
        host.with_fork_rom::<(), _>(&mut occupied, |_| panic!("owned Host target"))
            .is_err()
    );
    host.analysis_rom = None;
    let before = encoded(&host.emulator);
    let mut fork = host.fork();
    assert!(
        host.execute_fork::<()>(&mut fork, |_| panic!("missing source"))
            .is_err()
    );
    assert_eq!(encoded(&host.emulator), before);
    assert!(rom_words(&mut fork).is_empty());
}

#[test]
fn wrong_analysis_host_revision_or_registry_rejects_before_the_loan() {
    for wrong_registry in [false, true] {
        let mut host = host();
        let allocation = asset(&mut host);
        let mut analysis = self::host();
        analysis.emulator = host.fork();
        analysis.analysis_rom = None;
        if wrong_registry {
            analysis.registry = HookRegistry::empty(Revision::Rev00);
        } else {
            analysis.revision = Revision::Rev00;
        }
        assert!(
            host.with_fork_rom::<(), _>(&mut analysis, |_| panic!("wrong revision"))
                .is_err()
        );
        assert!(rom_words(&mut analysis.emulator).is_empty());
        assert_eq!(asset(&mut host), allocation);
    }
}

#[test]
fn borrowed_observation_peeks_preserve_rom_ram_alias_odd_and_wrapped_bytes() {
    let mut host = host();
    host.context().write_u32(0xff9001, 0x89ab_cdef);
    let before = encoded(&host.emulator);
    assert_eq!(abi::peek_u8(&host.emulator, 0x300), 0x12);
    assert_eq!(abi::peek_u8(&host.emulator, 0x301), 0x34);
    assert_eq!(abi::peek_u8(&host.emulator, 0x0100_0301), 0x34);
    for (offset, expected) in [0x89, 0xab, 0xcd, 0xef].into_iter().enumerate() {
        assert_eq!(
            abi::peek_u8(&host.emulator, 0xff9001 + offset as u32),
            expected
        );
        assert_eq!(
            abi::peek_u8(&host.emulator, 0xe09001 + offset as u32),
            expected
        );
    }
    let invalid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        abi::peek_u8(&host.emulator, 0xc00004)
    }));
    assert!(
        invalid.is_err(),
        "hardware still needs the explicit bus path"
    );
    assert_eq!(encoded(&host.emulator), before);
}

#[test]
#[ignore = "requires the two private validated cartridges; constructor/ownership only"]
fn validated_hosts_copy_the_analysis_asset_once_only_when_opted_in() {
    for (revision, variable) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = std::fs::read(std::env::var(variable).unwrap()).unwrap();
        for mode in [Mode::Reference, Mode::Replace] {
            let host = Host::new(&bytes, mode, HookRegistry::empty(revision)).unwrap();
            assert!(
                host.analysis_rom.is_none(),
                "no unused default analysis allocation"
            );
        }
        let mut host = Host::new(&bytes, Mode::Check, HookRegistry::empty(revision)).unwrap();
        assert!(host.analysis_rom.is_some());
        let before = encoded(&host.emulator);
        let mut fork = host.fork();
        let pointer = host
            .execute_fork(&mut fork, |fork| {
                let cartridge = fork.hybrid_bus().cartridge.as_ref().unwrap();
                assert_eq!(cartridge.rom_len(), bytes.len());
                for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
                    assert_eq!(
                        cartridge.peek_word((index * 2) as u32),
                        u16::from_be_bytes([pair[0], pair[1]])
                    );
                }
                Ok(rom_pointer(fork))
            })
            .unwrap();
        host.execute_fork(&mut fork, |fork| {
            assert_eq!(rom_pointer(fork), pointer, "same cached allocation");
            Ok(())
        })
        .unwrap();
        assert_eq!(encoded(&host.emulator), before);
    }
}

#[cfg(feature = "reference-proof-hooks")]
mod origin;
