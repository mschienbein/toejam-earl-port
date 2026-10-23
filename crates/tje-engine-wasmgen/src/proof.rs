//! Opt-in development harness. The interpreter/reference are not default dependencies.
mod audio;
mod cache;
mod instructions;
use serde::Serialize;
use tje_engine_core::{
    Bus, CpuState, InstructionIr, Program, ReadKind,
    ir::{BinaryOp, CompareOp, ExceptionExit, Op, StateField, Value, Width},
    metadata::{compile_program_with_divider, game_metadata},
};
use tje_engine_runtime::Backend;
use tje_engine_verify::{
    Machine, ReplayReport, interpreter::Interpreter, reference::ReferenceMachine,
    runtime::RuntimeMachine, verify_instruction_session,
};
use tje_rom::Rom;
use tje_session::Session;
use wasm_bindgen::prelude::*;

use crate::browser::now;
use crate::{BrowserBackend, CompilationMetrics};

fn failure(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[derive(Serialize)]
struct Metrics {
    cache_hit: bool,
    cache_read_ms: f64,
    cache_write_ms: f64,
    generation_ms: f64,
    instantiate_ms: f64,
    module_bytes: usize,
    storage_error: Option<String>,
}
impl From<&CompilationMetrics> for Metrics {
    fn from(value: &CompilationMetrics) -> Self {
        Self {
            cache_hit: value.cache_hit,
            cache_read_ms: value.cache_read_ms,
            cache_write_ms: value.cache_write_ms,
            generation_ms: value.generation_ms,
            instantiate_ms: value.instantiate_ms,
            module_bytes: value.module_bytes,
            storage_error: value.storage_error.clone(),
        }
    }
}
#[derive(Serialize)]
struct SessionProof {
    replay: ReplayReport,
    cold: Metrics,
    warm: Metrics,
    comparison_ms: f64,
    capacity_frames: u64,
    capacity_ms: f64,
    capacity_fps: f64,
}

/// Run a real retail session against a fresh reference emulator. Both images are
/// imported from the user's bytes. Generated modules and snapshots stay private.
#[wasm_bindgen]
pub async fn browser_verify_session(
    rom_bytes: Vec<u8>,
    session_json: String,
) -> Result<String, JsValue> {
    let rom = Rom::import(&rom_bytes).map_err(failure)?;
    let session: Session = serde_json::from_str(&session_json).map_err(failure)?;
    session.validate().map_err(failure)?;
    if tje_rom::Revision::from(session.revision) != rom.revision() {
        return Err(failure("session revision does not match imported ROM"));
    }
    let m68k_divider = 7;
    let cold = BrowserBackend::load(rom.revision().sha1(), m68k_divider, || {
        compile_program_with_divider(rom.bytes(), &game_metadata(rom.revision()), m68k_divider)
    })
    .await
    .map_err(failure)?;
    let cold_metrics = Metrics::from(cold.metrics());
    drop(cold);
    // A real IndexedDB hit must bypass both ROM decoding and IR generation.
    let backend = BrowserBackend::load(
        rom.revision().sha1(),
        m68k_divider,
        || -> Result<Program, &str> {
            Err("expected a warm IndexedDB module; regeneration must not run")
        },
    )
    .await
    .map_err(failure)?;
    let warm = Metrics::from(backend.metrics());
    let mut candidate = RuntimeMachine::new(&rom, backend, "wasm");
    let mut reference = ReferenceMachine::new(&rom);
    let start = now();
    let replay =
        verify_instruction_session(&session, &mut reference, &mut candidate).map_err(failure)?;
    let comparison_ms = now() - start;
    drop(reference);
    let mut capacity_frames = 0;
    let mut capacity_ms = 0.0;
    if replay.passed() {
        // Production devices and generated code, without proof snapshots/trace.
        candidate.engine_mut().reset();
        candidate.engine_mut().devices_mut().disable_trace();
        candidate
            .engine_mut()
            .devices_mut()
            .disable_instruction_trace();
        candidate
            .engine_mut()
            .devices_mut()
            .disable_ram_write_trace();
        let start = now();
        for (frame, pads) in session.inputs().enumerate() {
            for write in session
                .writes()
                .iter()
                .filter(|write| write.before_frame == frame as u64)
            {
                candidate.write_ram(write).map_err(failure)?;
            }
            candidate
                .engine_mut()
                .play_frame(tje_engine_verify::reference::inputs(pads))
                .map_err(failure)?;
            capacity_frames += 1;
        }
        capacity_ms = now() - start;
    }
    let result = SessionProof {
        replay,
        cold: cold_metrics,
        warm,
        comparison_ms,
        capacity_frames,
        capacity_ms,
        capacity_fps: if capacity_ms > 0.0 {
            capacity_frames as f64 * 1000.0 / capacity_ms
        } else {
            0.0
        },
    };
    serde_json::to_string(&result).map_err(failure)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Read(u32, u32, u32),
    Write(u32, u32, u32),
    Offset(u32),
}
#[derive(Clone, PartialEq, Eq)]
struct FixtureBus {
    bytes: [u8; 256],
    events: Vec<Event>,
}
impl FixtureBus {
    fn new(seed: u32) -> Self {
        Self {
            bytes: std::array::from_fn(|index| (index as u32).wrapping_mul(seed) as u8),
            events: Vec::new(),
        }
    }
}
impl Bus for FixtureBus {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        self.events.push(Event::Read(address, 1, kind as u32));
        self.bytes[(address & 255) as usize]
    }
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        self.events.push(Event::Read(address, 2, kind as u32));
        u16::from_be_bytes([
            self.bytes[(address & 255) as usize],
            self.bytes[(address.wrapping_add(1) & 255) as usize],
        ])
    }
    fn write8(&mut self, address: u32, value: u8) {
        self.events.push(Event::Write(address, 1, u32::from(value)));
        self.bytes[(address & 255) as usize] = value;
    }
    fn write16(&mut self, address: u32, value: u16) {
        self.events.push(Event::Write(address, 2, u32::from(value)));
        let [high, low] = value.to_be_bytes();
        self.bytes[(address & 255) as usize] = high;
        self.bytes[(address.wrapping_add(1) & 255) as usize] = low;
    }
    fn set_read_offset_mclk(&mut self, offset: u32) {
        self.events.push(Event::Offset(offset));
    }
}
fn fixture(pc: u32, operation: Op) -> InstructionIr {
    InstructionIr {
        pc,
        next_pc: pc + 2,
        opcode: 0,
        value_count: 5,
        ops: vec![
            Op::LoadState {
                dst: Value(0),
                field: StateField::Data(0),
            },
            Op::LoadState {
                dst: Value(1),
                field: StateField::Data(1),
            },
            operation,
            Op::StoreState {
                field: StateField::Data(2),
                value: Value(2),
            },
            Op::Constant {
                dst: Value(3),
                value: pc + 2,
            },
            Op::StoreState {
                field: StateField::Pc,
                value: Value(3),
            },
            Op::Constant {
                dst: Value(4),
                value: 0x8000_0040,
            },
            Op::Finish { cycles: Value(4) },
        ],
    }
}
fn fixture_program() -> Program {
    let mut instructions = Vec::new();
    for op in [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Xor,
        BinaryOp::ShiftLeft,
        BinaryOp::ShiftRight,
        BinaryOp::ArithmeticShiftRight,
        BinaryOp::DivideUnsigned,
        BinaryOp::DivideSigned,
        BinaryOp::RemainderUnsigned,
        BinaryOp::RemainderSigned,
    ] {
        let pc = 0x100 + instructions.len() as u32 * 2;
        instructions.push(fixture(
            pc,
            Op::Binary {
                dst: Value(2),
                op,
                lhs: Value(0),
                rhs: Value(1),
            },
        ));
    }
    for op in [
        CompareOp::Equal,
        CompareOp::NotEqual,
        CompareOp::UnsignedLess,
        CompareOp::UnsignedLessEqual,
        CompareOp::SignedLess,
        CompareOp::SignedLessEqual,
    ] {
        let pc = 0x100 + instructions.len() as u32 * 2;
        instructions.push(fixture(
            pc,
            Op::Compare {
                dst: Value(2),
                op,
                lhs: Value(0),
                rhs: Value(1),
            },
        ));
    }
    let pc = 0x100 + instructions.len() as u32 * 2;
    instructions.push(fixture(
        pc,
        Op::Select {
            dst: Value(2),
            condition: Value(0),
            if_true: Value(0),
            if_false: Value(1),
        },
    ));
    for width in [Width::Byte, Width::Word] {
        for kind in [ReadKind::Semantic, ReadKind::Prefetch, ReadKind::Discarded] {
            let pc = 0x100 + instructions.len() as u32 * 2;
            let mut ir = fixture(
                pc,
                Op::ReadMemoryIf {
                    dst: Value(2),
                    condition: Value(1),
                    address: Value(0),
                    width,
                    kind,
                },
            );
            // Ordered side effects before the conditional read are observable.
            ir.ops.insert(2, Op::SetReadOffset { cycles: Value(1) });
            ir.ops.insert(
                3,
                Op::WriteMemory {
                    address: Value(0),
                    value: Value(1),
                    width,
                },
            );
            instructions.push(ir);
        }
    }
    let pc = 0x100 + instructions.len() as u32 * 2;
    let mut ir = fixture(
        pc,
        Op::ReadMemory {
            dst: Value(2),
            address: Value(0),
            width: Width::Word,
            kind: ReadKind::Semantic,
        },
    );
    ir.ops.insert(
        3,
        Op::ExitIf {
            condition: Value(1),
            exit: ExceptionExit {
                vector: 3,
                cycles: Value(0),
                saved_pc: Value(1),
                fault_address: Value(0),
                fault_status: Value(2),
            },
        },
    );
    instructions.push(ir);
    Program { instructions }
}
fn random(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}
#[derive(Serialize)]
struct PrimitiveProof {
    cases: u64,
    functions: usize,
    generation: Metrics,
    execution_ms: f64,
}

/// Authored primitive-IR fixtures isolate backend errors from decoder errors.
/// Directed edge cases include MIN/-1 division, counts 0/31/32/63, conditional
/// reads, unsigned cycle packing, ordered bus effects and early exception exits.
#[wasm_bindgen]
pub async fn browser_verify_primitives() -> Result<String, JsValue> {
    let program = fixture_program();
    let mut backend = BrowserBackend::uncached(&program).await.map_err(failure)?;
    let mut interpreter = Interpreter::default();
    let directed = [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX, 31, 32, 63];
    let mut seed = 0x1357_2468;
    let mut cases = 0;
    let start = now();
    for ir in &program.instructions {
        for sample in 0..1088 {
            let lhs = if sample < 64 {
                directed[sample / 8]
            } else {
                random(&mut seed)
            };
            let mut rhs = if sample < 64 {
                directed[sample % 8]
            } else {
                random(&mut seed)
            };
            let division = ir.ops.iter().any(|op| {
                matches!(
                    op,
                    Op::Binary {
                        op: BinaryOp::DivideSigned
                            | BinaryOp::DivideUnsigned
                            | BinaryOp::RemainderSigned
                            | BinaryOp::RemainderUnsigned,
                        ..
                    }
                )
            });
            if division && rhs == 0 {
                rhs = 1;
            }
            let mut expected = CpuState {
                d: std::array::from_fn(|_| random(&mut seed)),
                a: std::array::from_fn(|_| random(&mut seed)),
                usp: random(&mut seed),
                ssp: random(&mut seed),
                pc: ir.pc,
                sr: random(&mut seed),
                prefetch: random(&mut seed),
                pending_interrupt: 0,
                stopped: 0,
                frozen: 0,
                address_error: 0,
                last_muldiv: random(&mut seed),
            };
            expected.d[0] = lhs;
            expected.d[1] = rhs;
            let mut actual = expected.clone();
            let mut expected_bus = FixtureBus::new(seed | 1);
            let mut actual_bus = expected_bus.clone();
            let expected_result = interpreter
                .evaluate(ir, &mut expected, &mut expected_bus)
                .map_err(failure)?;
            let actual_result = backend
                .execute(&mut actual, &mut actual_bus)
                .map_err(failure)?;
            if expected != actual || expected_result != actual_result || expected_bus != actual_bus
            {
                return Err(failure(format!(
                    "primitive mismatch at {:x} sample {sample}: expected {expected:?}/{expected_result:?}, actual {actual:?}/{actual_result:?}",
                    ir.pc
                )));
            }
            cases += 1;
        }
    }
    for pc in [0, 0x101, 0x200, u32::MAX] {
        let mut state = CpuState {
            pc,
            ..CpuState::default()
        };
        let original = state.clone();
        let mut bus = FixtureBus::new(1);
        if backend.execute(&mut state, &mut bus).is_ok()
            || state != original
            || !bus.events.is_empty()
        {
            return Err(failure(
                "missing continuation must fail without side effects",
            ));
        }
    }
    serde_json::to_string(&PrimitiveProof {
        cases,
        functions: program.instructions.len(),
        generation: backend.metrics().into(),
        execution_ms: now() - start,
    })
    .map_err(failure)
}
