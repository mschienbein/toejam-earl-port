//! The production first-launch path shared by the desktop frontend and measurement example.
use crate::{CacheConfig, CompileProgress, NativeProgram, Result};
use genesis_config::GenesisEmulatorConfig;
use sha1::{Digest, Sha1};
use tje_engine_core::metadata::{compile_program_with_divider, game_metadata};
use tje_mem::AddressSpace;
use tje_native_platform::DeviceRuntime;
use tje_rom::Rom;

/// The play API is owned by the backend-independent runtime, not duplicated here.
pub type Engine = tje_engine_runtime::Engine<NativeProgram>;

/// Lift only committed code metadata and the verified user's ROM, load or compile
/// the complete native program, and start the device-only runtime at reset.
///
/// Run this synchronous operation on the frontend's loading worker. Progress can
/// be sent over its existing UI channel. The returned engine exposes `play_frame`,
/// `frame_buffer`, `target_fps`, `set_audio_sample_rate`, `reset`, and `frame_count`.
/// `engine.backend().stats()` reports actual compilation/cache measurements.
pub fn load_engine(
    rom: Rom,
    config: &GenesisEmulatorConfig,
    cache: &CacheConfig,
    mut progress: impl FnMut(CompileProgress),
) -> Result<Engine> {
    progress(CompileProgress::Lifting);
    let revision = rom.revision();
    let rom_sha1: [u8; 20] = Sha1::digest(rom.bytes()).into();
    let program = compile_program_with_divider(
        rom.bytes(),
        &game_metadata(revision),
        config.clamped_m68k_divider().get() as u32,
    )?;
    let backend =
        NativeProgram::load_or_compile_with_progress(rom_sha1, &program, cache, &mut progress)?;
    // The IR and user's ROM are not copied into the executable object. The one
    // ROM allocation moves into the same address space used by every device.
    drop(program);
    let memory = AddressSpace::new(rom.into_bytes(), revision);
    Ok(Engine::from_devices(
        DeviceRuntime::new(memory, config),
        backend,
    ))
}
