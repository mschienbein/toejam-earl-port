use std::ffi::c_void;
use std::fmt;

use js_sys::{Object, Reflect, Uint8Array, WebAssembly::Table};
use tje_engine_core::{Bus, CpuState, ExceptionOut, Program, StepResult};
use tje_engine_runtime::Backend;
use tje_engine_runtime::abi::{BUS_CALLBACKS, BusContext};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::{ENGINE_VERSION, GenerateError, generate};

#[wasm_bindgen(module = "/src/browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = cachedBytes)]
    fn cached_bytes(sha1: &str, version: &str, m68k_divider: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = storeBytes)]
    pub(crate) fn store_bytes(
        sha1: &str,
        version: &str,
        m68k_divider: u32,
        bytes: &Uint8Array,
    ) -> js_sys::Promise;
    #[wasm_bindgen(js_name = evictBytes)]
    fn evict_bytes(sha1: &str, version: &str, m68k_divider: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = forgetRom)]
    fn forget_rom_bytes(sha1: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = instantiateBytes)]
    fn instantiate_bytes(bytes: &Uint8Array, imports: &Object) -> js_sys::Promise;
    #[wasm_bindgen(js_name = now)]
    pub(crate) fn now() -> f64;
}

/// Observed setup times, in browser performance-clock milliseconds.
#[derive(Debug, Clone, Default)]
pub struct CompilationMetrics {
    pub cache_hit: bool,
    pub cache_read_ms: f64,
    pub cache_write_ms: f64,
    /// Includes IR generation on a miss; zero on a verified cache hit.
    pub generation_ms: f64,
    pub instantiate_ms: f64,
    pub module_bytes: usize,
    /// Browser storage policy can disable persistence without disabling Wasm.
    pub storage_error: Option<String>,
}

#[derive(Debug)]
pub enum BrowserError {
    Configuration(&'static str),
    Generate(GenerateError),
    Program(String),
    Link(String),
    Module(String),
    MissingContinuation(u32),
}
impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => {
                write!(f, "invalid browser engine configuration: {error}")
            }
            Self::Generate(error) => error.fmt(f),
            Self::Program(error) => write!(f, "cannot generate browser IR: {error}"),
            Self::Link(error) => write!(f, "cannot link browser engine: {error}"),
            Self::Module(error) => write!(f, "cannot instantiate browser module: {error}"),
            Self::MissingContinuation(pc) => write!(f, "no compiled continuation at {pc:#x}"),
        }
    }
}
impl std::error::Error for BrowserError {}
fn link(error: JsValue) -> BrowserError {
    BrowserError::Link(format!("{error:?}"))
}
fn module(error: JsValue) -> BrowserError {
    let name = Reflect::get(&error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string());
    if matches!(
        name.as_deref(),
        Some("CompileError" | "LinkError" | "RuntimeError")
    ) {
        BrowserError::Module(format!("{error:?}"))
    } else {
        link(error)
    }
}
fn property(object: &JsValue, name: &str) -> Result<JsValue, BrowserError> {
    Reflect::get(object, &JsValue::from_str(name)).map_err(link)
}
fn set(object: &Object, name: &str, value: &JsValue) -> Result<(), BrowserError> {
    Reflect::set(object, &JsValue::from_str(name), value).map_err(link)?;
    Ok(())
}

type Entry = unsafe extern "C" fn(*mut CpuState, *mut c_void, *mut ExceptionOut) -> u64;

// Clear partially initialized entries on module errors as well as normal Drop.
struct EntrySlot {
    table: Table,
    index: u32,
}
impl Drop for EntrySlot {
    fn drop(&mut self) {
        let _ = self.table.set_raw(self.index, &JsValue::NULL);
    }
}

/// Executable continuations linked directly to the application's memory and bus.
/// Calls stay within Wasm: no JavaScript boundary, BigInt conversion, allocation,
/// or opcode dispatch is involved in `execute`.
pub struct BrowserBackend {
    entry: Entry,
    _slot: EntrySlot,
    // The slot is cleared before the generated instance is released.
    _instance: JsValue,
    metrics: CompilationMetrics,
}
impl BrowserBackend {
    /// Delete all stored compiled modules for this verified cartridge, across
    /// engine versions and clock dividers. Drop any live engine separately.
    pub async fn forget_rom(rom_sha1: &str) -> Result<(), BrowserError> {
        JsFuture::from(forget_rom_bytes(rom_sha1))
            .await
            .map_err(link)?;
        Ok(())
    }

    /// Read and verify IndexedDB before invoking the IR builder. The caller
    /// supplies the verified ROM SHA-1 and the actual divider used by the IR
    /// builder (normally seven). Both, plus ENGINE_VERSION, bind cached bytes.
    /// A verified usable cache hit never invokes the builder. A rejected cache
    /// is evicted and generated exactly once; fresh compiler errors propagate.
    pub async fn load<E: fmt::Display>(
        rom_sha1: &str,
        m68k_divider: u32,
        build_ir: impl FnOnce() -> Result<Program, E>,
    ) -> Result<Self, BrowserError> {
        if rom_sha1.len() != 40
            || !rom_sha1
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(BrowserError::Configuration(
                "ROM SHA-1 must be forty lowercase hex digits",
            ));
        }
        if m68k_divider == 0 {
            return Err(BrowserError::Configuration("68000 divider must be nonzero"));
        }
        let mut metrics = CompilationMetrics::default();
        let start = now();
        let cached = match JsFuture::from(cached_bytes(rom_sha1, ENGINE_VERSION, m68k_divider))
            .await
        {
            Ok(value) if !value.is_null() => Some(value.dyn_into::<Uint8Array>().map_err(link)?),
            Ok(_) => None,
            Err(error) => {
                metrics.storage_error = Some(format!("{error:?}"));
                None
            }
        };
        metrics.cache_read_ms = now() - start;
        if let Some(bytes) = cached {
            metrics.cache_hit = true;
            metrics.module_bytes = bytes.length() as usize;
            match Self::instantiate(&bytes, metrics.clone()).await {
                Ok(backend) => return Ok(backend),
                Err(BrowserError::Module(_)) => {
                    if let Err(error) =
                        JsFuture::from(evict_bytes(rom_sha1, ENGINE_VERSION, m68k_divider)).await
                    {
                        metrics.storage_error = Some(format!("{error:?}"));
                    }
                    metrics.cache_hit = false;
                }
                Err(error) => return Err(error),
            }
        }
        let start = now();
        let program = build_ir().map_err(|error| BrowserError::Program(error.to_string()))?;
        let generated = generate(&program).map_err(BrowserError::Generate)?;
        // Async JS compilation/storage must not retain a view that a later
        // Rust memory.grow can detach. Own one JS-side copy instead.
        let bytes = Uint8Array::from(generated.bytes.as_slice());
        metrics.generation_ms = now() - start;
        metrics.module_bytes = bytes.length() as usize;
        let mut backend = Self::instantiate(&bytes, metrics).await?;
        let start = now();
        if let Err(error) =
            JsFuture::from(store_bytes(rom_sha1, ENGINE_VERSION, m68k_divider, &bytes)).await
        {
            backend.metrics.storage_error = Some(format!("{error:?}"));
        }
        backend.metrics.cache_write_ms = now() - start;
        Ok(backend)
    }

    /// Compile an uncached program for bounded synthetic/oracle fixtures.
    pub async fn uncached(program: &Program) -> Result<Self, BrowserError> {
        let start = now();
        let generated = generate(program).map_err(BrowserError::Generate)?;
        let bytes = Uint8Array::from(generated.bytes.as_slice());
        let metrics = CompilationMetrics {
            generation_ms: now() - start,
            module_bytes: bytes.length() as usize,
            ..CompilationMetrics::default()
        };
        Self::instantiate(&bytes, metrics).await
    }

    async fn instantiate(
        bytes: &Uint8Array,
        mut metrics: CompilationMetrics,
    ) -> Result<Self, BrowserError> {
        std::hint::black_box(BUS_CALLBACKS);
        let table: Table = wasm_bindgen::function_table().unchecked_into();
        let index = table.grow(1).map_err(link)?;
        let slot = EntrySlot { table, index };
        let env = Object::new();
        set(&env, "memory", &wasm_bindgen::memory())?;
        set(&env, "table", &slot.table)?;
        set(&env, "entry_slot", &JsValue::from(index))?;
        let exports = wasm_bindgen::exports();
        for (import, export) in [
            ("read8", "tje_engine_bus_read8"),
            ("read16", "tje_engine_bus_read16"),
            ("write8", "tje_engine_bus_write8"),
            ("write16", "tje_engine_bus_write16"),
            ("set_read_offset", "tje_engine_bus_set_read_offset_mclk"),
        ] {
            let callback = property(&exports, export)?;
            if !callback.is_function() {
                return Err(BrowserError::Link(format!("missing raw export {export}")));
            }
            set(&env, import, &callback)?;
        }
        let imports = Object::new();
        set(&imports, "env", &env)?;
        let compiled = JsFuture::from(instantiate_bytes(bytes, &imports))
            .await
            .map_err(module)?;
        metrics.instantiate_ms = property(&compiled, "instantiateMs")?
            .as_f64()
            .ok_or_else(|| BrowserError::Link("missing instantiate measurement".into()))?;
        let instance = property(&compiled, "instance")?;
        // SAFETY: the generated module places its ENTRY_TYPE function at this
        // freshly reserved app-table slot. Its signature is exactly Entry. The
        // instance and EntrySlot stay alive for the whole backend lifetime.
        let entry = unsafe { std::mem::transmute::<u32, Entry>(index) };
        Ok(Self {
            entry,
            _slot: slot,
            _instance: instance,
            metrics,
        })
    }

    pub fn metrics(&self) -> &CompilationMetrics {
        &self.metrics
    }
}
impl Backend for BrowserBackend {
    type Error = BrowserError;

    fn execute(
        &mut self,
        cpu: &mut CpuState,
        bus: &mut dyn Bus,
    ) -> Result<StepResult, Self::Error> {
        let mut context = BusContext::new(bus);
        let mut exception = ExceptionOut::default();
        // SAFETY: layouts are shared, and the exclusive context lives for this
        // synchronous call. The module cannot retain its borrowed pointers.
        let packed = unsafe { (self.entry)(cpu, context.as_mut_ptr(), &mut exception) };
        let vector = (packed >> 32) as u32;
        if vector == u32::MAX {
            return Err(BrowserError::MissingContinuation(cpu.pc));
        }
        Ok(StepResult {
            cycles: packed as u32,
            vector,
            saved_pc: exception.saved_pc,
            fault_address: exception.fault_address,
            fault_status: exception.status_word,
        })
    }
}
