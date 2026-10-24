//! Desktop compiler for the cycle-exact engine. Each IR instruction becomes a
//! relocation-free native continuation, returning at the original interrupt boundary.
//!
//! Generated code and its cache are private derivatives of the user's ROM. They
//! are never built into the game or distributed. This crate contains no 68000
//! interpreter; it lowers Core's primitive SSA and calls only the device bus.
//!
//! # Safety boundary
//! The application must protect its per-user data directory from other writers.
//! Cache checksums detect accidental damage, not malicious replacement by a user
//! who can already modify application data. Bus callbacks must not unwind: like
//! other `extern "C"` callbacks, a panic aborts rather than crossing native frames.
#![deny(unsafe_op_in_unsafe_fn)]

mod cache;
#[cfg(feature = "devices")]
mod engine;
mod lower;

use cache::{Artifact, Identity};
pub use cache::{CacheConfig, ENGINE_VERSION};
use cranelift_codegen::{
    isa::TargetIsa,
    settings::{self, Configurable},
};
#[cfg(feature = "devices")]
pub use engine::{Engine, load_engine};
use memmap2::{Mmap, MmapMut};
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{Duration, Instant},
};
use tje_engine_core::{Bus, CpuState, ExceptionOut, Program, StepResult};
use tje_engine_runtime::abi::{BUS_CALLBACKS, BusCallbacks, BusContext};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("native compiler target: {0}")]
    Target(String),
    #[error("ROM instruction lifting: {0}")]
    FrontEnd(#[from] tje_engine_core::CompileError),
    #[error("invalid IR at {pc:#010x}: {message}")]
    InvalidIr { pc: u32, message: &'static str },
    #[error("native compilation at {pc:#010x}: {message}")]
    Compile { pc: u32, message: String },
    #[error("invalid native cache: {0}")]
    InvalidCache(&'static str),
    #[error("native code exceeds the cache's four-gigabyte address space")]
    CodeTooLarge,
    #[error("no per-user data directory; supply CacheConfig::data_dir")]
    NoUserDataDirectory,
    #[error("no compiled continuation for PC {0:#010x}")]
    MissingContinuation(u32),
    #[error("native code publication: {0}")]
    Publication(String),
    #[error("native code cache: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheStatus {
    Disabled,
    Miss,
    Hit,
    Rebuilt,
}

/// Synchronous first-launch notifications. Send these to a UI event channel;
/// never run a window/event loop inside the compiler callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileProgress {
    Lifting,
    CheckingCache,
    Compiling { completed: usize, total: usize },
    WritingCache,
    Ready { cache_status: CacheStatus },
}

/// Observed startup costs. Compilation excludes serialization and disk flushing;
/// startup includes identity checks, loading/compiling, cache publication and W^X mapping.
#[derive(Clone, Debug)]
pub struct CompileStats {
    pub instruction_count: usize,
    pub code_bytes: usize,
    pub cache_bytes: u64,
    pub compilation_time: Duration,
    pub startup_time: Duration,
    pub cache_status: CacheStatus,
    pub cache_path: Option<PathBuf>,
    pub cache_rejection: Option<String>,
}

/// An immutable executable mapping and a direct PC-indexed continuation table.
/// No ROM, IR, decoder, or semantic interpreter is retained after construction.
pub struct NativeProgram {
    code: Mmap,
    offsets: Vec<u32>,
    stats: CompileStats,
    rom_sha1: [u8; 20],
    program_sha256: [u8; 32],
}

impl NativeProgram {
    /// Compile an entire already-validated Core program without a persistent cache.
    pub fn compile(rom_sha1: [u8; 20], program: &Program) -> Result<Self> {
        Self::create(rom_sha1, program, None, &mut |_| {})
    }

    /// Load code only after checking the ROM, IR, engine, ISA, feature flags,
    /// checksum and continuation layout. A damaged/stale cache is regenerated.
    pub fn load_or_compile(
        rom_sha1: [u8; 20],
        program: &Program,
        cache: &CacheConfig,
    ) -> Result<Self> {
        Self::load_or_compile_with_progress(rom_sha1, program, cache, |_| {})
    }

    pub fn load_or_compile_with_progress(
        rom_sha1: [u8; 20],
        program: &Program,
        cache: &CacheConfig,
        mut progress: impl FnMut(CompileProgress),
    ) -> Result<Self> {
        Self::create(rom_sha1, program, Some(cache), &mut progress)
    }

    fn create(
        rom_sha1: [u8; 20],
        program: &Program,
        cache: Option<&CacheConfig>,
        progress: &mut impl FnMut(CompileProgress),
    ) -> Result<Self> {
        let start = Instant::now();
        validate_program(program)?;
        let isa = native_isa()?;
        let identity = Identity {
            rom_sha1,
            program_sha256: program_hash(program),
            target: target_fingerprint(&*isa),
        };
        let path = cache.map(|cache| cache.path(&identity));
        let mut stats = CompileStats {
            instruction_count: program.instructions.len(),
            code_bytes: 0,
            cache_bytes: 0,
            compilation_time: Duration::ZERO,
            startup_time: Duration::ZERO,
            cache_status: CacheStatus::Disabled,
            cache_path: path.clone(),
            cache_rejection: None,
        };
        if let Some(path) = &path {
            progress(CompileProgress::CheckingCache);
            match cache::load(path, &identity).and_then(|artifact| {
                artifact.validate(
                    program
                        .instructions
                        .iter()
                        .map(|instruction| instruction.pc),
                )?;
                Ok(artifact)
            }) {
                Ok(artifact) => {
                    stats.cache_status = CacheStatus::Hit;
                    stats.cache_bytes = std::fs::metadata(path)?.len();
                    let program =
                        Self::map(artifact, stats, start, rom_sha1, identity.program_sha256)?;
                    progress(CompileProgress::Ready {
                        cache_status: CacheStatus::Hit,
                    });
                    return Ok(program);
                }
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    stats.cache_status = CacheStatus::Miss;
                }
                Err(Error::InvalidCache(reason)) => {
                    stats.cache_status = CacheStatus::Rebuilt;
                    stats.cache_rejection = Some(reason.into());
                }
                Err(error) => return Err(error),
            }
        }
        let compile_start = Instant::now();
        let artifact = lower::compile(program, &*isa, |completed, total| {
            progress(CompileProgress::Compiling { completed, total });
        })?;
        stats.compilation_time = compile_start.elapsed();
        artifact.validate(
            program
                .instructions
                .iter()
                .map(|instruction| instruction.pc),
        )?;
        if let Some(path) = &path {
            progress(CompileProgress::WritingCache);
            stats.cache_bytes = cache::store(path, &identity, &artifact)?;
        }
        let program = Self::map(artifact, stats, start, rom_sha1, identity.program_sha256)?;
        progress(CompileProgress::Ready {
            cache_status: program.stats.cache_status,
        });
        Ok(program)
    }

    fn map(
        artifact: Artifact,
        mut stats: CompileStats,
        start: Instant,
        rom_sha1: [u8; 20],
        program_sha256: [u8; 32],
    ) -> Result<Self> {
        stats.code_bytes = artifact.code.len();
        // Anonymous RW memory is populated only after cache verification, then
        // changed to RX. The cache file itself is never executable or mapped live.
        let mut code = MmapMut::map_anon(artifact.code.len())?;
        code.copy_from_slice(&artifact.code);
        let code = code.make_exec()?;
        // RX permissions alone do not synchronize instruction caches. Use the
        // matching Cranelift/Wasmtime platform implementation, including Darwin's
        // sys_icache_invalidate and Windows' FlushInstructionCache.
        // SAFETY: this is the live mapping just populated above. It remains
        // immutable and no executable pointer escapes before both maintenance
        // operations complete, including cross-thread instruction-pipeline sync.
        unsafe {
            wasmtime_internal_jit_icache_coherence::clear_cache(code.as_ptr().cast(), code.len())
                .map_err(|error| Error::Publication(error.to_string()))?;
        }
        wasmtime_internal_jit_icache_coherence::pipeline_flush_mt()
            .map_err(|error| Error::Publication(error.to_string()))?;
        let last_pc = artifact
            .entries
            .last()
            .expect("validated nonempty artifact")
            .pc;
        let mut offsets = vec![u32::MAX; (last_pc / 2) as usize + 1];
        for entry in artifact.entries {
            offsets[(entry.pc / 2) as usize] = entry.offset;
        }
        stats.startup_time = start.elapsed();
        Ok(Self {
            code,
            offsets,
            stats,
            rom_sha1,
            program_sha256,
        })
    }

    pub fn stats(&self) -> &CompileStats {
        &self.stats
    }

    pub fn contains(&self, pc: u32) -> bool {
        pc & 1 == 0
            && self
                .offsets
                .get((pc / 2) as usize)
                .is_some_and(|&offset| offset != u32::MAX)
    }

    /// Execute exactly one original instruction. The runtime, not the backend,
    /// advances devices, consumes stalls and accepts interrupts between calls.
    pub fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult> {
        let pc = cpu.pc;
        let offset = self
            .offsets
            .get((pc / 2) as usize)
            .copied()
            .filter(|&offset| pc & 1 == 0 && offset != u32::MAX)
            .ok_or(Error::MissingContinuation(pc))?;
        let mut context = BusContext::new(bus);
        let mut exception = ExceptionOut::default();
        type Continuation = unsafe extern "C" fn(
            *mut CpuState,
            *mut c_void,
            *const BusCallbacks,
            *mut ExceptionOut,
        ) -> u64;
        // SAFETY: offsets are validated instruction starts, and all code was emitted
        // for this exact ABI/ISA or loaded from the current user's verified cache.
        // The RX mapping outlives the call. Every pointer refers to a live, exclusive
        // state/context/output or immutable callback table for this synchronous call.
        let packed = unsafe {
            let entry: Continuation = std::mem::transmute(self.code.as_ptr().add(offset as usize));
            entry(cpu, context.as_mut_ptr(), &BUS_CALLBACKS, &mut exception)
        };
        let vector = (packed >> 32) as u32;
        Ok(StepResult {
            cycles: packed as u32,
            vector,
            saved_pc: exception.saved_pc,
            fault_address: exception.fault_address,
            fault_status: exception.status_word,
        })
    }
}

impl tje_engine_runtime::Backend for NativeProgram {
    type Error = Error;
    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult> {
        NativeProgram::execute(self, cpu, bus)
    }
}

impl tje_engine_runtime::portable_timing::TimingBackend for NativeProgram {
    fn contains_compiled_instruction(&self, pc: u32) -> bool {
        self.contains(pc)
    }
    fn compiled_identity(&self) -> ([u8; 20], [u8; 32]) {
        (self.rom_sha1, self.program_sha256)
    }
    fn authenticates_compilation(&self, rom_sha1: [u8; 20], program: &Program) -> bool {
        self.rom_sha1 == rom_sha1 && self.program_sha256 == program_hash(program)
    }
}

fn native_isa() -> Result<std::sync::Arc<dyn TargetIsa>> {
    let mut flags = settings::builder();
    flags
        .set("opt_level", "speed")
        .map_err(|error| Error::Target(error.to_string()))?;
    flags
        .set("enable_verifier", "true")
        .map_err(|error| Error::Target(error.to_string()))?;
    let builder = cranelift_native::builder().map_err(|error| Error::Target(error.into()))?;
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|error| Error::Target(error.to_string()))
}

fn target_fingerprint(isa: &dyn TargetIsa) -> String {
    use std::fmt::Write;
    let mut target = format!("{};{}", isa.triple(), isa.flags());
    for setting in isa.isa_flags() {
        write!(&mut target, ";{setting}").expect("writing to String cannot fail");
    }
    target
}

fn validate_program(program: &Program) -> Result<()> {
    let mut previous = None;
    for instruction in &program.instructions {
        if instruction.pc & 1 != 0
            || instruction.pc > 0x00ff_ffff
            || previous.is_some_and(|previous| instruction.pc <= previous)
        {
            return Err(Error::InvalidIr {
                pc: instruction.pc,
                message: "PCs must be sorted, unique, even 24-bit addresses",
            });
        }
        previous = Some(instruction.pc);
    }
    if previous.is_none() {
        return Err(Error::InvalidIr {
            pc: 0,
            message: "empty program",
        });
    }
    Ok(())
}

fn program_hash(program: &Program) -> [u8; 32] {
    struct ProgramHasher(Sha256);
    impl Hasher for ProgramHasher {
        fn write(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }
        fn finish(&self) -> u64 {
            u64::from_le_bytes(
                self.0.clone().finalize()[..8]
                    .try_into()
                    .expect("eight-byte prefix"),
            )
        }
    }
    let mut hash = ProgramHasher(Sha256::new());
    program.hash(&mut hash);
    hash.0.finalize().into()
}
