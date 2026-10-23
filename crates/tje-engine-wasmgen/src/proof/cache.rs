//! Authored continuations exercising the actual BrowserBackend cache path.
use js_sys::Uint8Array;
use std::cell::Cell;
use tje_engine_core::{
    CpuState, InstructionIr, Program,
    ir::{Op, StateField, Value},
};
use tje_engine_runtime::Backend;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Event, FixtureBus, failure};
use crate::{BrowserBackend, ENGINE_VERSION, browser::store_bytes};

#[wasm_bindgen(module = "/src/proof/cache.js")]
extern "C" {
    #[wasm_bindgen(js_name = tamperRecord)]
    fn tamper_record(sha1: &str, version: &str, divider: u32, mode: &str) -> js_sys::Promise;
}

fn program(divider: u32) -> Program {
    Program {
        instructions: vec![InstructionIr {
            pc: 0x100,
            next_pc: 0x102,
            opcode: 0,
            value_count: 4,
            ops: vec![
                Op::Constant {
                    dst: Value(0),
                    value: 42,
                },
                Op::StoreState {
                    field: StateField::Data(0),
                    value: Value(0),
                },
                Op::Constant {
                    dst: Value(1),
                    value: divider * 4,
                },
                Op::SetReadOffset { cycles: Value(1) },
                Op::Constant {
                    dst: Value(2),
                    value: 0x102,
                },
                Op::StoreState {
                    field: StateField::Pc,
                    value: Value(2),
                },
                Op::Constant {
                    dst: Value(3),
                    value: 4,
                },
                Op::Finish { cycles: Value(3) },
            ],
        }],
    }
}
fn check(backend: &mut BrowserBackend, divider: u32) -> Result<(), JsValue> {
    let mut cpu = CpuState {
        pc: 0x100,
        ..CpuState::default()
    };
    let mut bus = FixtureBus::new(1);
    let result = backend.execute(&mut cpu, &mut bus).map_err(failure)?;
    if cpu.d[0] != 42
        || cpu.pc != 0x102
        || result.cycles != 4
        || result.vector != 0
        || bus.events != [Event::Offset(divider * 4)]
    {
        return Err(failure(
            "cached continuation or divider-dependent read compensation changed",
        ));
    }
    Ok(())
}
async fn generated_once(sha1: &str, divider: u32) -> Result<BrowserBackend, JsValue> {
    let builds = Cell::new(0);
    let mut backend = BrowserBackend::load(sha1, divider, || {
        builds.set(builds.get() + 1);
        Ok::<_, &str>(program(divider))
    })
    .await
    .map_err(failure)?;
    if builds.get() != 1 || backend.metrics().cache_hit {
        return Err(failure("cache rejection must regenerate exactly once"));
    }
    check(&mut backend, divider)?;
    Ok(backend)
}
async fn warm(sha1: &str, divider: u32) -> Result<BrowserBackend, JsValue> {
    let mut backend = BrowserBackend::load(sha1, divider, || -> Result<Program, &str> {
        Err("verified warm hit invoked IR builder")
    })
    .await
    .map_err(failure)?;
    if !backend.metrics().cache_hit || backend.metrics().generation_ms != 0.0 {
        return Err(failure("warm cache did not bypass IR generation"));
    }
    check(&mut backend, divider)?;
    Ok(backend)
}

#[wasm_bindgen]
pub async fn browser_verify_cache() -> Result<String, JsValue> {
    let sha1 = "2".repeat(40);
    drop(generated_once(&sha1, 7).await?);
    drop(warm(&sha1, 7).await?);
    let mut corruptions = Vec::new();
    for mode in ["semantic", "magic", "identity", "digest"] {
        let valid = JsFuture::from(tamper_record(&sha1, ENGINE_VERSION, 7, mode)).await?;
        if mode == "semantic" && valid.as_bool() != Some(true) {
            return Err(failure("semantic byte mutation must remain valid Wasm"));
        }
        drop(generated_once(&sha1, 7).await?);
        drop(warm(&sha1, 7).await?);
        corruptions.push(mode);
    }
    // A correctly hashed but structurally invalid artifact must also recover,
    // exercising actual WebAssembly CompileError handling, not only hashing.
    let invalid = Uint8Array::from(&[0_u8; 8][..]);
    JsFuture::from(store_bytes(&sha1, ENGINE_VERSION, 7, &invalid)).await?;
    drop(generated_once(&sha1, 7).await?);
    drop(warm(&sha1, 7).await?);
    // Both configurations coexist, and each invokes its own generated timing.
    drop(generated_once(&sha1, 8).await?);
    drop(warm(&sha1, 7).await?);
    drop(warm(&sha1, 8).await?);
    let builds = Cell::new(0);
    let failed = BrowserBackend::load(&sha1, 9, || -> Result<Program, &str> {
        builds.set(builds.get() + 1);
        Err("authored fresh compiler failure")
    })
    .await;
    if failed.is_ok() || builds.get() != 1 {
        return Err(failure("fresh compiler failure was suppressed or retried"));
    }
    serde_json::to_string(&serde_json::json!({ "warm_builder_bypassed": true,
        "corruptions_regenerated_once": corruptions, "hashed_invalid_module_regenerated_once": true,
        "divider_isolation": true, "fresh_error_propagated_once": true }))
    .map_err(failure)
}
