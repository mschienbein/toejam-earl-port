//! Genesis public interface and main loop
// TJE standalone cutover: remove CD/32X construction, firmware, and their
// coupled channel debugger. Standalone CPU/peripheral timing stays upstream.

use crate::audio::GenesisAudioResampler;
use crate::bus::GenesisBus;
use bincode::{Decode, Encode};
use genesis_components::cartridge::Cartridge;
use genesis_components::vdp::{Vdp, VdpTickEffect};
use genesis_config::{
    GenesisAspectRatio, GenesisButton, GenesisEmulatorConfig, GenesisInputs, GenesisRegion,
};
use jgenesis_common::frontend::{
    AudioOutput, EmulatorTrait, InputPoller, Modal, PartialClone, RenderFrameOptions, Renderer,
    SaveWriter, SuggestedWindowSize, TickEffect, TickResult, TimingMode,
};
use m68000_emu::M68000;
use std::fmt::{Debug, Display};
use thiserror::Error;
use z80_emu::Z80;

const SRAM_EXTENSION: &str = "sav";

#[derive(Debug, Error)]
pub enum GenesisError<RErr, AErr, SErr> {
    #[error("Rendering error: {0}")]
    Render(RErr),
    #[error("Audio output error: {0}")]
    Audio(AErr),
    #[error("Save write error: {0}")]
    Save(SErr),
}

pub type GenesisResult<RErr, AErr, SErr> = Result<TickEffect, GenesisError<RErr, AErr, SErr>>;

// TJE hybrid: one reference phase record shared by CPU and timing-only issue.
struct ReferenceAdvance {
    pc: u32,
    cycles: u32,
    waiting: bool,
    exception_return: bool,
}

#[derive(Debug, Encode, Decode, PartialClone)]
pub struct GenesisEmulator {
    m68k: M68000,
    z80: Z80,
    #[partial_clone(partial)]
    bus: GenesisBus,
    timing_mode: TimingMode,
    audio_resampler: GenesisAudioResampler,
    config: GenesisEmulatorConfig,
}

impl GenesisEmulator {
    /// TJE strict data-plane control encoding, by borrow. No machine clone,
    /// normalization, queue flush, RAM rewrite or device advancement occurs.
    /// Main RAM and the VDP have separate exact-domain/control/output checks.
    pub fn hybrid_encode_non_vdp_main_ram_control(
        &self,
    ) -> Result<Vec<u8>, bincode::error::EncodeError> {
        let Self { m68k, z80, bus, timing_mode, audio_resampler, config } = self;
        bincode::encode_to_vec(
            (
                m68k,
                z80,
                bus.hybrid_control_without_vdp_main_ram(),
                timing_mode,
                audio_resampler,
                config,
            ),
            bincode::config::standard(),
        )
    }

    // TJE hybrid accessors: retain the upstream instruction/timing loop.
    pub fn hybrid_parts(&mut self) -> (&mut M68000, &mut GenesisBus) {
        (&mut self.m68k, &mut self.bus)
    }

    pub fn hybrid_cpu(&self) -> &M68000 {
        &self.m68k
    }

    pub fn hybrid_bus(&self) -> &GenesisBus {
        &self.bus
    }

    pub fn hybrid_bus_mut(&mut self) -> &mut GenesisBus {
        &mut self.bus
    }

    /// TJE native: fork the retained devices and data, not the 68000 engine.
    pub fn capture_native_state(&self) -> crate::native::PeripheralState {
        use z80_emu::BusInterface;
        let bus = &self.bus;
        let mut address = [0; 8];
        address[..7].copy_from_slice(&self.m68k.address_registers());
        address[7] = self.m68k.stack_pointer();
        crate::native::PeripheralState {
            ram: (0..0x10000).map(|a| bus.memory.read_main_ram_byte(a)).collect(),
            audio_ram: (0..0x2000).map(|a| bus.memory.read_audio_ram(a)).collect(),
            vdp: bus.vdp.clone(),
            ym2612: bus.ym2612.clone(),
            psg: bus.psg.clone(),
            input: bus.input.clone(),
            z80: self.z80.clone(),
            z80_bank: bus.z80_bank.value(),
            bus_request: bus.busreq(),
            reset: bus.reset(),
            cycles: bus.cycles,
            timing_mode: self.timing_mode,
            region: bus.cartridge.as_ref().map_or(GenesisRegion::Americas, Cartridge::region),
            audio_resampler: self.audio_resampler.clone(),
            config: self.config.clone(),
            program_counter: self.m68k.pc(),
            native_input_remainder: 0,
            open_bus: bus.open_bus,
            data: self.m68k.data_registers(),
            address,
            status: self.m68k.status_register(),
            user_stack_pointer: self.m68k.user_stack_pointer(),
            supervisor_stack_pointer: self.m68k.supervisor_stack_pointer(),
        }
    }

    pub fn create<S: SaveWriter>(
        mut cartridge_rom: Option<Vec<u8>>,
        config: GenesisEmulatorConfig,
        save_writer: &mut S,
    ) -> Self {
        log::info!("Running standalone Genesis hardware");

        if cartridge_rom.as_ref().is_some_and(Vec::is_empty) {
            // Later code assumes cartridge ROM is non-empty if Some
            cartridge_rom = None;
        }

        let initial_ram = save_writer.load_bytes(SRAM_EXTENSION).ok();

        let cartridge = cartridge_rom
            .map(|rom| Cartridge::new(rom, initial_ram, config.forced_region, &config.cheat_codes));

        let region = cartridge.as_ref().map(Cartridge::region).unwrap_or(GenesisRegion::Americas);

        let timing_mode = config.forced_timing_mode.unwrap_or(match region {
            GenesisRegion::Europe => TimingMode::Pal,
            GenesisRegion::Americas | GenesisRegion::Japan => TimingMode::Ntsc,
        });

        log::info!("Using timing / display mode {timing_mode}");

        let bus = GenesisBus::new(timing_mode, cartridge, &config);

        // The Genesis does not allow TAS to lock the bus, so don't allow TAS writes
        let m68k = M68000::builder().allow_tas_writes(false).name("Main".into()).build();
        let z80 = Z80::new();

        let audio_resampler = GenesisAudioResampler::new(timing_mode, &config);

        let mut emulator = Self { m68k, z80, bus, timing_mode, audio_resampler, config };

        // Reset CPU so that execution will start from the right place
        emulator.bus.m68k_reset = true;
        emulator.m68k.execute_instruction(&mut emulator.bus);
        emulator.bus.m68k_reset = false;

        emulator
    }

    #[must_use]
    pub fn game_title(&mut self) -> Option<String> {
        self.bus.game_title()
    }

    #[inline]
    #[must_use]
    pub fn has_sram(&self) -> bool {
        self.bus.has_persistent_ram()
    }

    #[inline]
    #[must_use]
    pub fn timing_mode(&self) -> TimingMode {
        self.timing_mode
    }

    fn render_frame<R: Renderer>(&mut self, renderer: &mut R) -> Result<(), R::Err> {
        render_frame(self.timing_mode, &self.bus.vdp, &self.config, renderer)
    }

    /// TJE timing-only entry: publish input/opcode attribution before Rust
    /// performs this instruction's real reads and queues its writes.
    pub fn hybrid_begin_timing_instruction(&mut self, inputs: GenesisInputs) {
        self.bus.input.set_inputs(inputs);
        self.bus.m68k_opcode = self.m68k.next_opcode();
        self.bus.timing_m68k(self.m68k.pc());
    }

    /// TJE timing-only completion. No original 68000 instruction is executed.
    /// Call only with a dependency-checked instruction grant, after Rust issued
    /// all effects. Uses the identical retained device/commit/output tail.
    pub fn hybrid_advance_timing_cycles<R, A, S>(
        &mut self,
        pc: u32,
        cpu_cycles: u32,
        waiting: bool,
        renderer: &mut R,
        audio_output: &mut A,
        save_writer: &mut S,
    ) -> TickResult<GenesisError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        S: SaveWriter,
    {
        let exception_return =
            !waiting && self.bus.timing_survey.is_some() && self.bus.m68k_opcode == 0x4e73;
        self.advance_devices(
            ReferenceAdvance { pc, cycles: cpu_cycles, waiting, exception_return },
            renderer,
            audio_output,
            save_writer,
        )
    }

    #[inline]
    fn tick_inner<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
    ) -> TickResult<GenesisError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<GenesisInputs>,
        S: SaveWriter,
    {
        self.bus.input.set_inputs(*input_poller.poll());
        self.bus.m68k_opcode = self.m68k.next_opcode();

        let m68k_pc = self.m68k.pc();
        self.bus.timing_m68k(m68k_pc);
        let timing_rte = self.bus.timing_survey.is_some()
            && self.bus.m68k_opcode == 0x4E73
            && self.m68k.hybrid_instruction_ready(&self.bus);
        let m68k_wait = self.bus.cycles.m68k_wait_cpu_cycles != 0;
        let m68k_cycles = if m68k_wait {
            self.bus.cycles.take_m68k_wait_cpu_cycles()
        } else {
            self.m68k.execute_instruction(&mut self.bus)
        };

        self.advance_devices(
            ReferenceAdvance {
                pc: m68k_pc,
                cycles: m68k_cycles,
                waiting: m68k_wait,
                exception_return: timing_rte,
            },
            renderer,
            audio_output,
            save_writer,
        )
    }

    fn advance_devices<R, A, S>(
        &mut self,
        advance: ReferenceAdvance,
        renderer: &mut R,
        audio_output: &mut A,
        save_writer: &mut S,
    ) -> TickResult<GenesisError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        S: SaveWriter,
    {
        let ReferenceAdvance {
            pc: m68k_pc,
            cycles: m68k_cycles,
            waiting: m68k_wait,
            exception_return: timing_rte,
        } = advance;

        let elapsed_mclk_cycles = self.bus.cycles.record_68k_instruction(
            m68k_pc,
            m68k_cycles,
            m68k_wait,
            self.bus.vdp.should_halt_cpu(),
        );

        while self.bus.cycles.should_tick_z80() {
            if !self.bus.cycles.z80_halt {
                self.bus.timing_z80(self.z80.pc());
                self.z80.tick(&mut self.bus);
            }
            self.bus.cycles.z80_cycle();
        }

        self.bus.timing_m68k(m68k_pc);
        let vdp_tick_effect = self.bus.tick_components(
            m68k_cycles,
            elapsed_mclk_cycles,
            m68k_wait,
            &mut self.audio_resampler,
        );
        if timing_rte && !m68k_wait {
            self.bus.timing_return();
        }

        self.audio_resampler.output_samples(audio_output).map_err(GenesisError::Audio)?;

        if vdp_tick_effect == VdpTickEffect::FrameComplete {
            self.render_frame(renderer).map_err(GenesisError::Render)?;

            self.persist_save_files(save_writer).map_err(GenesisError::Save)?;
        }

        Ok(match vdp_tick_effect {
            VdpTickEffect::FrameComplete => TickEffect::FrameRendered,
            VdpTickEffect::None => TickEffect::None,
        })
    }

    fn persist_save_files<S: SaveWriter>(&mut self, save_writer: &mut S) -> Result<(), S::Err> {
        if !self.bus.has_persistent_ram() || !self.bus.get_and_clear_persistent_ram_dirty() {
            return Ok(());
        }

        if let Some(ram) = self.bus.cartridge().map(Cartridge::external_ram)
            && !ram.is_empty()
        {
            save_writer.persist_bytes(SRAM_EXTENSION, ram)?;
        }

        Ok(())
    }
}

fn render_frame<R: Renderer>(
    timing_mode: TimingMode,
    vdp: &Vdp,
    config: &GenesisEmulatorConfig,
    renderer: &mut R,
) -> Result<(), R::Err> {
    let frame_size = vdp.frame_size();
    let pixel_aspect_ratio = config.aspect_ratio.to_pixel_aspect_ratio(
        timing_mode,
        frame_size,
        config.to_gen_par_params(),
    );
    let target_fps = genesis_components::target_framerate(vdp, timing_mode);

    renderer.render_frame(
        vdp.frame_buffer(),
        frame_size,
        target_fps,
        RenderFrameOptions {
            pixel_aspect_ratio,
            composite_params: Some(vdp.composite_params()),
            ..RenderFrameOptions::default()
        },
    )
}

impl EmulatorTrait for GenesisEmulator {
    type Button = GenesisButton;
    type Inputs = GenesisInputs;
    type Config = GenesisEmulatorConfig;
    type SaveState = Self;

    type Err<
        RErr: Debug + Display + Send + Sync + 'static,
        AErr: Debug + Display + Send + Sync + 'static,
        SErr: Debug + Display + Send + Sync + 'static,
    > = GenesisError<RErr, AErr, SErr>;

    /// Execute one 68000 CPU instruction and run the rest of the components for the appropriate
    /// number of cycles.
    ///
    /// # Errors
    ///
    /// This method will propagate any errors encountered while rendering frames or pushing audio
    /// samples.
    #[inline]
    fn tick<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
    ) -> GenesisResult<R::Err, A::Err, S::Err>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<Self::Inputs>,
        S: SaveWriter,
    {
        self.tick_inner(renderer, audio_output, input_poller, save_writer)
    }

    fn force_render<R>(&mut self, renderer: &mut R) -> Result<(), R::Err>
    where
        R: Renderer,
    {
        self.render_frame(renderer)
    }

    fn reload_config(&mut self, config: &Self::Config) {
        self.bus.reload_config(config);
        self.audio_resampler.reload_config(self.timing_mode, config);

        self.config = config.clone();
    }

    fn soft_reset(&mut self) {
        log::info!("Soft resetting console");

        self.bus.reset();

        self.bus.m68k_reset = true;
        self.m68k.execute_instruction(&mut self.bus);
        self.bus.m68k_reset = false;
    }

    fn hard_reset<S: SaveWriter>(&mut self, save_writer: &mut S) {
        log::info!("Hard resetting console");

        let cartridge_rom = self.bus.cartridge_mut().map(Cartridge::take_rom);

        *self = GenesisEmulator::create(cartridge_rom, self.config.clone(), save_writer);
    }

    fn load_state(&mut self, mut state: Self::SaveState) {
        if let Some(state_cartridge) = state.bus.cartridge_mut()
            && let Some(self_cartridge) = self.bus.cartridge_mut()
        {
            state_cartridge.take_rom_from(self_cartridge);
        }

        *self = state;
    }

    fn to_save_state(&self) -> Self::SaveState {
        self.partial_clone()
    }

    fn target_fps(&self) -> f64 {
        genesis_components::target_framerate(&self.bus.vdp, self.timing_mode)
    }

    fn update_audio_output_frequency(&mut self, output_frequency: u64) {
        self.audio_resampler.update_output_frequency(output_frequency);
    }

    fn startup_modals(&self) -> Vec<Modal> {
        let mut modals = Vec::new();

        if self.config.remove_sprite_limits
            && self
                .bus
                .cartridge
                .as_ref()
                .is_some_and(|cartridge| cartridge.metadata().sprite_limit_compatibility_issues)
        {
            modals.push(Modal {
                id: None,
                text: genesis_components::SPRITE_LIMITS_MODAL_MESSAGE.into(),
            });
        }

        modals
    }

    fn suggested_window_size(&self) -> SuggestedWindowSize {
        let frame_size = self.bus.vdp.frame_size();
        let par_params = self.config.to_gen_par_params();
        let aspect_ratio = self.config.aspect_ratio.to_pixel_aspect_ratio(
            self.timing_mode,
            frame_size,
            par_params,
        );
        let aspect_ratio_with_fallback = aspect_ratio.or(GenesisAspectRatio::default()
            .to_pixel_aspect_ratio(self.timing_mode, frame_size, par_params));

        let mut width: f64 = frame_size.width.into();
        let mut height: f64 = frame_size.height.into();

        if let Some(aspect_ratio) = aspect_ratio_with_fallback {
            width *= aspect_ratio.get();
        }

        if frame_size.height >= 448 {
            // In interlaced mode; cut suggested window size dimensions in half
            width *= 0.5;
            height *= 0.5;
        }

        SuggestedWindowSize { width, height, fallback_aspect_ratio: aspect_ratio.is_none() }
    }
}

#[cfg(test)]
mod hybrid_control_tests {
    use super::*;

    fn machine() -> GenesisEmulator {
        let config = GenesisEmulatorConfig::default();
        let timing_mode = TimingMode::Ntsc;
        let m68k = M68000::builder()
            .allow_tas_writes(false)
            .name("Authored control fixture".into())
            .build();
        let z80 = Z80::new();
        let bus = GenesisBus::new(timing_mode, None, &config);
        let audio_resampler = GenesisAudioResampler::new(timing_mode, &config);
        GenesisEmulator { m68k, z80, bus, timing_mode, audio_resampler, config }
    }

    #[test]
    fn full_cpu_z80_resampler_config_and_bus_remain_in_control_encoding() {
        type Mutation = (&'static str, fn(&mut GenesisEmulator));
        let mutations: &[Mutation] = &[
            ("cpu", |m| m.m68k.set_data_registers([1, 0, 0, 0, 0, 0, 0, 0])),
            ("z80", |m| m.z80.set_pc(7)),
            ("bus", |m| m.bus.open_bus ^= 1),
            ("timing_mode", |m| m.timing_mode = TimingMode::Pal),
            ("resampler", |m| m.audio_resampler.update_output_frequency(32000)),
            ("config", |m| m.config.cheat_codes.push((0xff0010, 7))),
        ];
        let baseline = machine().hybrid_encode_non_vdp_main_ram_control().unwrap();
        for &(name, mutate) in mutations {
            let mut changed = machine();
            mutate(&mut changed);
            assert_ne!(
                baseline,
                changed.hybrid_encode_non_vdp_main_ram_control().unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn encoding_never_rewrites_state_and_excludes_only_explicit_data_planes() {
        let mut machine = machine();
        let before = bincode::encode_to_vec(&machine, bincode::config::standard()).unwrap();
        let baseline = machine.hybrid_encode_non_vdp_main_ram_control().unwrap();
        assert_eq!(before, bincode::encode_to_vec(&machine, bincode::config::standard()).unwrap());
        machine.bus.memory.write_main_ram_word(0xff0010, 0xabcd);
        machine.bus.vdp.write_control(0x4000);
        assert_eq!(baseline, machine.hybrid_encode_non_vdp_main_ram_control().unwrap());
        assert_ne!(before, bincode::encode_to_vec(&machine, bincode::config::standard()).unwrap());
    }
}
