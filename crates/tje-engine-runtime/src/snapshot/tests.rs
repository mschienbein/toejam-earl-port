use super::*;
use crate::{Bus, StepResult};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use std::convert::Infallible;
use tje_mem::{AddressSpace, Revision};
use tje_native_platform::DeviceRuntime;

// Authored bus workload, not game code or a retail-state fixture.
struct Workload;
impl Backend for Workload {
    type Error = Infallible;
    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult, Infallible> {
        cpu.d[0] = cpu.d[0].wrapping_add(1);
        bus.write16(0xff0100, cpu.d[0] as u16);
        bus.write8(0xc00011, 0x90 | (cpu.d[0] & 15) as u8);
        Ok(StepResult {
            cycles: 44,
            ..StepResult::default()
        })
    }
}
fn machine(marker: u8) -> Machine<PlatformDevices, Workload> {
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0xffff00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[0x100..0x102].copy_from_slice(&0x4e71_u16.to_be_bytes());
    rom[0x200] = marker;
    Machine::from_devices(
        DeviceRuntime::new(
            AddressSpace::new(rom, Revision::Rev02),
            &GenesisEmulatorConfig::default(),
        ),
        Workload,
    )
}
#[test]
fn restores_every_canonical_byte_and_continues_identically() {
    let mut engine = machine(0);
    for _ in 0..3 {
        engine.play_frame(GenesisInputs::default()).unwrap();
    }
    let checkpoint = engine.snapshot().unwrap();
    let decoded = MachineSnapshot::from_bytes(checkpoint.as_bytes().to_vec()).unwrap();
    let mut expected = Vec::new();
    for _ in 0..4 {
        engine.play_frame(GenesisInputs::default()).unwrap();
        expected.push(engine.snapshot().unwrap());
    }
    engine.restore(&decoded).unwrap();
    assert_eq!(engine.snapshot().unwrap().as_bytes(), checkpoint.as_bytes());
    for expected in expected {
        engine.play_frame(GenesisInputs::default()).unwrap();
        assert_eq!(engine.snapshot().unwrap().as_bytes(), expected.as_bytes());
    }
}
#[test]
fn rejects_rom_version_corruption_and_truncation_without_mutation() {
    let mut engine = machine(0);
    let before = engine.snapshot().unwrap();
    let wrong_rom = machine(1).snapshot().unwrap();
    assert_eq!(engine.restore(&wrong_rom), Err(SnapshotError::RomMismatch));
    let mut version = before.as_bytes().to_vec();
    version[8..12].copy_from_slice(&99_u32.to_le_bytes());
    assert_eq!(
        MachineSnapshot::from_bytes(version).unwrap_err(),
        SnapshotError::UnsupportedVersion(99)
    );
    let mut corrupt = before.as_bytes().to_vec();
    corrupt[200] ^= 1;
    assert_eq!(
        MachineSnapshot::from_bytes(corrupt).unwrap_err(),
        SnapshotError::Corrupt
    );
    assert_eq!(
        MachineSnapshot::from_bytes(before.as_bytes()[..100].to_vec()).unwrap_err(),
        SnapshotError::Corrupt
    );
    assert_eq!(engine.snapshot().unwrap().as_bytes(), before.as_bytes());
}
#[test]
fn reusable_capture_keeps_capacity_and_ring_discards_the_future_branch() {
    let engine = machine(0);
    let mut image = engine.snapshot().unwrap();
    let capacity = image.bytes.capacity();
    engine.snapshot_into(&mut image).unwrap();
    assert_eq!(image.bytes.capacity(), capacity);
    let cost = capacity + std::mem::size_of::<MachineSnapshot>();
    let mut ring = RewindBuffer::new(2 * cost);
    for frame in 0_u64..5 {
        let mut copy = image.clone();
        copy.bytes[32..40].copy_from_slice(&frame.to_le_bytes());
        ring.push(copy);
        assert!(ring.used_bytes() <= 2 * cost);
    }
    assert!(ring.len() <= 2);
    assert_eq!(ring.pop_before(4).unwrap().frame_count(), 3);
    assert!(ring.is_empty());
    ring.clear();
    assert_eq!(ring.used_bytes(), 0);
}

#[test]
fn execution_profile_identity_is_checked_before_any_mutation() {
    let mut engine = machine(0);
    let faithful = engine.snapshot().unwrap();
    let enhanced = engine
        .snapshot_with_profile("authored-enhanced:v1/config-a")
        .unwrap();
    assert_eq!(
        engine.restore(&enhanced),
        Err(SnapshotError::ProfileMismatch)
    );
    assert_eq!(engine.snapshot().unwrap().as_bytes(), faithful.as_bytes());
    assert_eq!(
        engine.restore_with_profile(&faithful, "authored-enhanced:v1/config-a"),
        Err(SnapshotError::ProfileMismatch)
    );
    engine
        .restore_with_profile(&enhanced, "authored-enhanced:v1/config-a")
        .unwrap();
    assert_eq!(
        engine
            .snapshot_with_profile("authored-enhanced:v1/config-a")
            .unwrap()
            .as_bytes(),
        enhanced.as_bytes()
    );
}
