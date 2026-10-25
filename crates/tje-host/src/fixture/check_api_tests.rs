//! Authored ROM-free API/observer tests, never cartridge or campaign approval.
use super::*;
use crate::{CheckReport, Mode, abi::HookContext, hooks::HookRegistry};
use tje_game::{call_frame::FrameState, execution::CallCeiling, exits::ExitContract, ids};

#[derive(Clone, Copy, Debug)]
enum Api {
    Call,
    Fixture,
    Entry,
    Clocked,
    Continuation,
}
impl Api {
    fn name(self) -> &'static str {
        match self {
            Self::Call => "check_call",
            Self::Fixture => "check_fixture",
            Self::Entry => "check_entry_fixture",
            Self::Clocked => "check_clocked_fixture",
            Self::Continuation => "check_continuation_fixture",
        }
    }
    fn invoke(self, host: &mut Host, address: u32) -> Result<()> {
        match self {
            Self::Call => host.check_call(address, &[0x12345678]).map(|_| ()),
            Self::Fixture => host.check_fixture(address, &[0x12345678]).map(|_| ()),
            Self::Entry => host.check_entry_fixture(address).map(|_| ()),
            Self::Clocked => host
                .check_clocked_fixture(address, &[0x12345678])
                .map(|_| ()),
            Self::Continuation => host
                .check_continuation_fixture(
                    ids::GET_RANDOM_NUMBER,
                    tje_game::exits::MainLoopStage::Begin,
                    continuation,
                    FrameState::new(0xff7000),
                )
                .map(|_| ()),
        }
    }
}
const APIS: [Api; 5] = [
    Api::Call,
    Api::Fixture,
    Api::Entry,
    Api::Clocked,
    Api::Continuation,
];

fn continuation(_: &mut dyn tje_game::Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    panic!("invalid fixture mode/contract must not dispatch a continuation")
}
fn retained_registers(_: &mut HookContext<'_>) -> Result<Option<u32>> {
    Ok(None)
}
fn host(mode: Mode) -> Host {
    let mut host = tests::synthetic_host(&[0x4e75], &[0x4e73]);
    host.mode = mode;
    if mode == Mode::Reference {
        // Match the real Reference constructor: no readable-root capability.
        host.readable_stack = None;
    }
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredFixtureRts";
    symbol.addresses = [0x100; 2];
    symbol.sizes = [2; 2];
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    let mut registry = HookRegistry::empty(host.revision);
    registry
        .register_exit_contract(
            symbol,
            tje_game::execution::ExecutionClass::Atomic,
            ExitContract::Rts,
            retained_registers,
        )
        .unwrap();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    host
}
fn machine(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    machine: Vec<u8>,
    checks: Vec<u8>,
    guard: Option<Vec<u8>>,
    frame: u64,
    clock_frame: u64,
    clock_usage: tje_game::execution::ClockUsage,
    clock_active: bool,
}
fn state(host: &Host) -> Snapshot {
    let guard = host.readable_stack.as_ref().map(|pool| {
        serde_json::to_vec(&crate::readable_stack::ReadableGuardMetrics::from(
            pool.namespace.metrics(),
        ))
        .unwrap()
    });
    Snapshot {
        machine: machine(host),
        checks: serde_json::to_vec(&host.checks).unwrap(),
        guard,
        frame: host.frame,
        clock_frame: host.wait_clock.frame,
        clock_usage: host.wait_clock.call_clock.usage(),
        clock_active: host.wait_clock.call_clock.active(),
    }
}

#[test]
fn non_check_modes_reject_before_prefix_boundary_isolation_seed_or_accounting() {
    for mode in [Mode::Reference, Mode::Replace] {
        for api in APIS {
            for wait_cycles in [0, 20] {
                let mut host = host(mode);
                // Mode is first on both call-ready and non-call-ready machines.
                host.emulator.hybrid_bus_mut().cycles.m68k_wait_cpu_cycles = wait_cycles;
                let before = state(&host);
                let error = api.invoke(&mut host, 0x100).unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!("invalid metadata: {} requires Check mode", api.name())
                );
                assert_eq!(before, state(&host), "{mode:?} {api:?} wait={wait_cycles}");
                assert_eq!(host.checks.calls, 0);
            }
        }
    }
}

#[test]
fn check_mode_errors_publish_current_guard_metrics_without_advancing_or_approving() {
    for api in APIS {
        let mut host = host(Mode::Check);
        assert!(!host.checks.readable_stack_guard.enabled);
        let before = machine(&host);
        let error = api.invoke(&mut host, 0x800000).unwrap_err();
        assert!(!error.to_string().contains("requires Check mode"));
        assert_eq!(before, machine(&host), "{api:?}: {error}");
        assert_eq!(host.checks.calls, 0);
        assert!(host.checks.readable_stack_guard.enabled);
        let actual = crate::readable_stack::ReadableGuardMetrics::from(
            host.readable_stack.as_ref().unwrap().namespace.metrics(),
        );
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(host.checks.readable_stack_guard).unwrap()
        );
    }
}

#[test]
fn successful_public_calls_publish_metrics_at_result_boundary() {
    for api in [Api::Call, Api::Fixture, Api::Entry] {
        let mut host = host(Mode::Check);
        if matches!(api, Api::Entry) {
            let sp = host.cpu().stack_pointer();
            host.context().write_u32(sp, 0x300);
        }
        assert!(!host.checks.readable_stack_guard.enabled);
        api.invoke(&mut host, 0x100).unwrap();
        assert_eq!(host.checks.calls, 1);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.readable_stack_guard.enabled);
        let actual = crate::readable_stack::ReadableGuardMetrics::from(
            host.readable_stack.as_ref().unwrap().namespace.metrics(),
        );
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(host.checks.readable_stack_guard).unwrap()
        );
    }
}

#[test]
fn reference_original_measurement_still_uses_generic_isolation_without_a_guard_pool() {
    let mut host = host(Mode::Reference);
    assert!(host.readable_stack.is_none());
    let entry = ids::MOVE_DATA_TO_VRAM.metadata().address(host.revision);
    host.emulator
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view()[entry as usize / 2] = 0x4e75; // Authored RTS.
    let before = state(&host);
    assert!(
        host.check_fixture(0x100, &[])
            .unwrap_err()
            .to_string()
            .contains("requires Check mode")
    );
    assert_eq!(before, state(&host));
    let measurement = host
        .measure_original_clocked_fixture(
            ids::MOVE_DATA_TO_VRAM,
            &[0x80000000, 0],
            CallCeiling {
                ticks: 100,
                frames: 1,
                reference_instructions: Some(10),
            },
        )
        .unwrap();
    assert!(!measurement.approves_translation);
    assert_eq!(measurement.usage_to_rts.main_thread_instructions, 1);
    assert_eq!(measurement.usage_to_rts.exception_handler_instructions, 0);
    assert_eq!(measurement.return_pc, measurement.caller_pc);
    assert_eq!(host.checks.calls, 0);
    assert!(host.readable_stack.is_none());
    assert_eq!(before, state(&host));
}
