//! Reference-only timing survey. Outputs metadata/counts, never game memory or pixels.
use anyhow::{Context, Result, ensure};
use bincode::Decode;
use clap::{Parser, ValueEnum};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
#[cfg(test)]
use genesis_core::timing_survey::TimingKind;
use genesis_core::timing_survey::{
    TimingAccess, TimingClass, TimingCpu, TimingPolicy, TimingSurvey,
};
use genesis_core::{GenesisEmulator, HybridWrite};
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, PartialClone, RenderFrameOptions,
    Renderer, SaveWriter, TickEffect,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, convert::Infallible, fs, io, path::PathBuf};
use tje_host::{catalogue, input::InputScript};
use tje_rom::Rom;

#[derive(Parser)]
#[command(about = "Measure original Genesis timing and independent single-frame retiming forks")]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    inventory: PathBuf,
    #[arg(long)]
    input: Option<PathBuf>,
    #[arg(long, default_value_t = 18000)]
    frames: u64,
    #[arg(long)]
    output: PathBuf,
    /// Fork each specified class from identical reference state, for both targets.
    #[arg(long, value_enum, value_delimiter = ',')]
    experiment: Vec<Experiment>,
    #[arg(long, default_value_t = 1200)]
    experiment_start: u64,
    #[arg(long, default_value_t = 600)]
    experiment_frames: u64,
    #[arg(long, default_value_t = 112)]
    fixed_scanline: u16,
    /// Record observations only in this half-open video-frame interval.
    #[arg(long, default_value_t = 0)]
    measure_start: u64,
    #[arg(long)]
    measure_end: Option<u64>,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Experiment {
    VdpControl,
    VdpData,
    VdpAll,
    Ym2612,
    Psg,
    SoundAll,
}
impl Experiment {
    fn class(self) -> TimingClass {
        match self {
            Self::VdpControl => TimingClass::VdpControl,
            Self::VdpData => TimingClass::VdpData,
            Self::VdpAll => TimingClass::VdpAll,
            Self::Ym2612 => TimingClass::Ym2612,
            Self::Psg => TimingClass::Psg,
            Self::SoundAll => TimingClass::SoundAll,
        }
    }
}
#[derive(Deserialize)]
struct Inventory {
    functions: Vec<Symbol>,
}
#[derive(Deserialize)]
struct Symbol {
    name: String,
    addresses: BTreeMap<String, String>,
    sizes: BTreeMap<String, u32>,
}
struct Symbols {
    ranges: Vec<(u32, u32, String)>,
    revision: String,
}
impl Symbols {
    fn load(path: &PathBuf, revision: &str) -> Result<Self> {
        let inventory: Inventory = serde_json::from_str(&fs::read_to_string(path)?)?;
        let mut ranges = Vec::new();
        for symbol in inventory.functions {
            let address = symbol
                .addresses
                .get(revision)
                .context("missing revision address")?;
            let start = u32::from_str_radix(address.trim_start_matches("0x"), 16)?;
            let size = *symbol
                .sizes
                .get(revision)
                .context("missing revision size")?;
            ranges.push((start, start + size, symbol.name));
        }
        ranges.sort_by_key(|r| r.0);
        Ok(Self {
            ranges,
            revision: revision.into(),
        })
    }
    fn at(&self, pc: u32) -> &str {
        let i = self.ranges.partition_point(|r| r.0 <= pc);
        if i == 0 || pc >= self.ranges[i - 1].1 {
            "unattributed"
        } else {
            &self.ranges[i - 1].2
        }
    }
    fn address(&self, name: &str) -> Result<u32> {
        self.ranges
            .iter()
            .find(|r| r.2 == name)
            .map(|r| r.0)
            .with_context(|| format!("inventory lacks {name}"))
    }
}
#[derive(Default)]
struct Sink {
    samples: Vec<[f64; 2]>,
}
impl Renderer for Sink {
    type Err = Infallible;
    fn render_frame(
        &mut self,
        _: &[Color],
        _: FrameSize,
        _: f64,
        _: RenderFrameOptions,
    ) -> Result<(), Self::Err> {
        Ok(())
    }
}
impl AudioOutput for Sink {
    type Err = Infallible;
    fn push_sample(&mut self, l: f64, r: f64) -> Result<(), Self::Err> {
        self.samples.push([l, r]);
        Ok(())
    }
}
struct Inputs(GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}
struct Saves;
impl SaveWriter for Saves {
    type Err = io::Error;
    fn load_bytes(&mut self, _: &str) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: Decode<()>>(&mut self, _: &str) -> io::Result<D> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: bincode::Encode>(&mut self, _: &str, _: E) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
#[derive(Default, Serialize)]
struct Count {
    accesses: u64,
    minimum_line_clock: Option<u16>,
    maximum_line_clock: u16,
}
impl Count {
    fn merge(&mut self, other: Self) {
        self.accesses += other.accesses;
        if let Some(minimum) = other.minimum_line_clock {
            self.minimum_line_clock = Some(self.minimum_line_clock.unwrap_or(minimum).min(minimum));
        }
        self.maximum_line_clock = self.maximum_line_clock.max(other.maximum_line_clock);
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AccessKey {
    pc: u32,
    z80: bool,
    kind: genesis_core::timing_survey::TimingKind,
    write: bool,
    active: bool,
    interrupt: u8,
    line: u16,
}
#[derive(Default, Serialize)]
struct WorkTick {
    start_frame: u64,
    end_frame: u64,
    start_clock: u64,
    elapsed_clocks: u64,
    elapsed_m68k_cycles: u64,
}
#[derive(Serialize)]
struct Lag {
    frame: u64,
    pc: u32,
    function: String,
    idle_draws: u64,
}
#[derive(Default, Serialize)]
struct Summary {
    schema_version: u32,
    revision: String,
    frames: u64,
    measured_frames: u64,
    /// function / CPU / kind / read-or-write / vertical-phase / interrupt / scanline
    accesses: BTreeMap<String, Count>,
    #[serde(skip)]
    raw_accesses: BTreeMap<AccessKey, Count>,
    #[serde(skip)]
    raw_registers: BTreeMap<(u32, u16, bool, u8, u16), u64>,
    entries: BTreeMap<String, u64>,
    first_entry: BTreeMap<String, u64>,
    work_ticks: Vec<WorkTick>,
    idle_draws_per_frame: Vec<u64>,
    /// Includes intentional modal waits: not a claim that every entry is lag.
    nonidle_vblanks: Vec<Lag>,
    hint_entries: BTreeMap<u16, u64>,
    hint_returns: BTreeMap<u16, u64>,
    hint_elapsed_clocks: Vec<u64>,
    maximum_tick_clocks: u64,
    vdp_register_writes: BTreeMap<String, u64>,
    experiments: BTreeMap<String, Comparison>,
}
#[derive(Default, Serialize)]
struct Comparison {
    frames: u64,
    retimed_writes: u64,
    maximum_delay_clocks: u64,
    pixel_different_frames: u64,
    different_pixels: u64,
    maximum_different_pixels: u64,
    first_pixel_difference: Option<u64>,
    audio_different_frames: u64,
    different_stereo_samples: u64,
    compared_stereo_samples: u64,
    maximum_sample_error: f64,
    sum_squared_sample_error: f64,
    first_audio_difference: Option<u64>,
    sound_register_stream_different_frames: u64,
    sound_register_count_difference: u64,
    ram_different_frames: u64,
}
impl Summary {
    fn observe(&mut self, access: TimingAccess) {
        let active = access.scanline < access.active_lines;
        if access.cpu == TimingCpu::M68k
            && access.kind == genesis_core::timing_survey::TimingKind::VdpControl
            && access.write
            && access.value & 0xC000 == 0x8000
        {
            let register = (access.value >> 8) & 0x1F;
            *self
                .raw_registers
                .entry((
                    access.pc,
                    register,
                    active,
                    access.interrupt,
                    access.scanline,
                ))
                .or_default() += 1;
        }
        let count = self
            .raw_accesses
            .entry(AccessKey {
                pc: access.pc,
                z80: access.cpu == TimingCpu::Z80,
                kind: access.kind,
                write: access.write,
                active,
                interrupt: access.interrupt,
                line: access.scanline,
            })
            .or_default();
        count.accesses += 1;
        count.minimum_line_clock = Some(
            count
                .minimum_line_clock
                .unwrap_or(access.line_clock)
                .min(access.line_clock),
        );
        count.maximum_line_clock = count.maximum_line_clock.max(access.line_clock);
    }

    fn finish(&mut self, symbols: &Symbols) {
        for (access, count) in std::mem::take(&mut self.raw_accesses) {
            let issuer = if access.z80 {
                format!("Z80Program@{:04X}", access.pc)
            } else {
                symbols.at(access.pc).to_owned()
            };
            let cpu = if access.z80 { "Z80" } else { "68000" };
            let phase = if access.active { "active" } else { "vblank" };
            let direction = if access.write { "write" } else { "read" };
            let key = format!(
                "{issuer}/{cpu}/{:?}/{direction}/{phase}/irq{}/{}",
                access.kind, access.interrupt, access.line
            );
            self.accesses.entry(key).or_default().merge(count);
        }
        for ((pc, register, active, interrupt, line), count) in
            std::mem::take(&mut self.raw_registers)
        {
            let function = symbols.at(pc);
            let phase = if active { "active" } else { "vblank" };
            *self
                .vdp_register_writes
                .entry(format!(
                    "{function}/R{register}/{phase}/irq{interrupt}/{line}"
                ))
                .or_default() += count;
        }
    }
}
fn ram_word(emulator: &GenesisEmulator, address: u32) -> u16 {
    emulator.hybrid_bus().memory.read_main_ram_word(address)
}
fn ram_long(emulator: &GenesisEmulator, address: u32) -> u32 {
    (u32::from(ram_word(emulator, address)) << 16) | u32::from(ram_word(emulator, address + 2))
}
fn clock(emulator: &GenesisEmulator) -> u64 {
    emulator.hybrid_bus().cycles.m68k_mclk_cycles
}
fn sound(writes: &[HybridWrite]) -> impl Iterator<Item = &HybridWrite> + Clone {
    writes
        .iter()
        .filter(|w| !matches!(w, HybridWrite::Vdp { .. }))
}
fn take_rom(destination: &mut GenesisEmulator, source: &mut GenesisEmulator) {
    destination
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .take_rom_from(source.hybrid_bus_mut().cartridge.as_mut().unwrap());
}
fn advance(emulator: &mut GenesisEmulator, inputs: GenesisInputs, audio: &mut Sink) -> Result<()> {
    audio.samples.clear();
    let mut inputs = Inputs(inputs);
    loop {
        let effect = emulator.tick(&mut Sink::default(), audio, &mut inputs, &mut Saves)?;
        if let Some(survey) = &mut emulator.hybrid_bus_mut().timing_survey {
            survey.accesses.clear();
        }
        if effect == TickEffect::FrameRendered {
            return Ok(());
        }
    }
}
fn compare(
    reference: &GenesisEmulator,
    altered: &GenesisEmulator,
    reference_audio: &Sink,
    altered_audio: &Sink,
    frame: u64,
    result: &mut Comparison,
) {
    result.frames += 1;
    let survey = altered.hybrid_bus().timing_survey.as_ref().unwrap();
    result.retimed_writes += survey.retimed_writes;
    result.maximum_delay_clocks = result.maximum_delay_clocks.max(survey.maximum_delay_clocks);
    let a = &reference.hybrid_bus().vdp;
    let b = &altered.hybrid_bus().vdp;
    let different = a
        .frame_buffer()
        .iter()
        .zip(b.frame_buffer())
        .filter(|(a, b)| a != b)
        .count() as u64
        + a.frame_buffer().len().abs_diff(b.frame_buffer().len()) as u64;
    if different != 0 || a.frame_size() != b.frame_size() {
        result.pixel_different_frames += 1;
        result.first_pixel_difference.get_or_insert(frame);
    }
    result.different_pixels += different;
    result.maximum_different_pixels = result.maximum_different_pixels.max(different);
    let mut different_samples = reference_audio
        .samples
        .len()
        .abs_diff(altered_audio.samples.len()) as u64;
    for (a, b) in reference_audio.samples.iter().zip(&altered_audio.samples) {
        if a[0].to_bits() != b[0].to_bits() || a[1].to_bits() != b[1].to_bits() {
            different_samples += 1;
        }
        for channel in 0..2 {
            let error = (a[channel] - b[channel]).abs();
            result.maximum_sample_error = result.maximum_sample_error.max(error);
            result.sum_squared_sample_error += error * error;
        }
    }
    result.compared_stereo_samples += reference_audio.samples.len() as u64;
    result.different_stereo_samples += different_samples;
    if different_samples != 0 {
        result.audio_different_frames += 1;
        result.first_audio_difference.get_or_insert(frame);
    }
    let a = sound(reference.hybrid_bus().hybrid_writes.as_ref().unwrap());
    let b = sound(altered.hybrid_bus().hybrid_writes.as_ref().unwrap());
    result.sound_register_count_difference += a.clone().count().abs_diff(b.clone().count()) as u64;
    if !a.eq(b) {
        result.sound_register_stream_different_frames += 1;
    }
    if (0..65_536)
        .step_by(2)
        .any(|address| ram_word(reference, address) != ram_word(altered, address))
    {
        result.ram_different_frames += 1;
    }
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.frames > 0, "frames must be positive");
    ensure!(
        args.fixed_scanline < 224,
        "fixed scanline must lie in active NTSC display"
    );
    let rom = Rom::import(&fs::read(&args.rom)?)?;
    let revision = match rom.revision() {
        tje_rom::Revision::Rev02 => "rev02",
        tje_rom::Revision::Rev00 => "rev00",
    };
    let symbols = Symbols::load(&args.inventory, revision)?;
    let script = if let Some(path) = &args.input {
        InputScript::parse(&fs::read_to_string(path)?)?
    } else {
        InputScript::default()
    };
    let mut emulator = GenesisEmulator::create(
        Some(rom.into_bytes()),
        GenesisEmulatorConfig::default(),
        &mut Saves,
    );
    emulator.hybrid_bus_mut().timing_survey = Some(TimingSurvey::default());
    emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
    let mut summary = Summary {
        schema_version: 1,
        revision: symbols.revision.clone(),
        frames: args.frames,
        ..Summary::default()
    };
    // Source-relative offsets are measured original loop labels, not ABI metadata.
    let further = symbols.address("FurtherInitialisation")?;
    let main_loop = further + (catalogue::MAIN_LOOP - 0x1286);
    let main_wait = further + (catalogue::MAIN_WAIT - 0x1286);
    let idle_return = further + (catalogue::IDLE_RANDOM_RETURN - 0x1286);
    let random = symbols.address("GetRandomNumber")?;
    let hint = symbols.address("Level4Isr")?;
    let mut audio = Sink::default();
    let mut alternate_audio = Sink::default();
    let mut idle = false;
    let mut game = false;
    let mut work = None;
    let mut hint_start = None;
    let mut forks = Vec::new();
    for frame in 0..args.frames {
        let measure =
            frame >= args.measure_start && frame < args.measure_end.unwrap_or(args.frames);
        let inputs = script.at(frame);
        let experiment = frame >= args.experiment_start
            && frame < args.experiment_start.saturating_add(args.experiment_frames);
        forks.clear();
        if experiment {
            for &class in &args.experiment {
                for line in [224, args.fixed_scanline] {
                    let mut fork = emulator.partial_clone();
                    let survey = fork.hybrid_bus_mut().timing_survey.as_mut().unwrap();
                    survey.policy = Some(TimingPolicy {
                        class: class.class(),
                        scanline: line,
                    });
                    survey.retimed_writes = 0;
                    survey.maximum_delay_clocks = 0;
                    fork.hybrid_bus_mut()
                        .hybrid_writes
                        .as_mut()
                        .unwrap()
                        .clear();
                    take_rom(&mut fork, &mut emulator);
                    advance(&mut fork, inputs, &mut alternate_audio)?;
                    take_rom(&mut emulator, &mut fork);
                    forks.push((
                        format!("{class:?}/line{line}"),
                        fork,
                        std::mem::take(&mut alternate_audio.samples),
                    ));
                }
            }
        }
        audio.samples.clear();
        emulator
            .hybrid_bus_mut()
            .hybrid_writes
            .as_mut()
            .unwrap()
            .clear();
        let mut inputs = Inputs(inputs);
        let mut idle_draws = 0;
        loop {
            let pc = emulator.hybrid_cpu().pc();
            let ready = emulator.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
                && emulator
                    .hybrid_cpu()
                    .hybrid_instruction_ready(emulator.hybrid_bus());
            if ready {
                if measure && symbols.ranges.binary_search_by_key(&pc, |r| r.0).is_ok() {
                    let name = symbols.at(pc);
                    if let Some(count) = summary.entries.get_mut(name) {
                        *count += 1;
                    } else {
                        summary.entries.insert(name.into(), 1);
                        summary.first_entry.insert(name.into(), frame);
                    }
                }
                if pc == main_loop {
                    game = true;
                    idle = false;
                    work = Some((frame, clock(&emulator)));
                }
                if pc == main_wait {
                    idle = true;
                    if let Some((start_frame, start_clock)) = work.take() {
                        let elapsed_clocks = clock(&emulator) - start_clock;
                        if measure {
                            summary.work_ticks.push(WorkTick {
                                start_frame,
                                end_frame: frame,
                                start_clock,
                                elapsed_clocks,
                                elapsed_m68k_cycles: elapsed_clocks / 7,
                            });
                        }
                    }
                }
                if pc == random
                    && ram_long(&emulator, emulator.hybrid_cpu().stack_pointer()) == idle_return
                {
                    idle_draws += 1;
                }
                if measure && pc == hint {
                    *summary
                        .hint_entries
                        .entry(emulator.hybrid_bus().vdp.scanline())
                        .or_default() += 1;
                    hint_start = Some(clock(&emulator));
                }
                if measure
                    && emulator.hybrid_cpu().next_opcode() == 0x4E73
                    && symbols.at(pc) == "Level4Isr"
                {
                    *summary
                        .hint_returns
                        .entry(emulator.hybrid_bus().vdp.scanline())
                        .or_default() += 1;
                    if let Some(start) = hint_start.take() {
                        summary.hint_elapsed_clocks.push(clock(&emulator) - start);
                    }
                }
            }
            let previous_line = emulator.hybrid_bus().vdp.scanline();
            let before_clock = clock(&emulator);
            let effect =
                emulator.tick(&mut Sink::default(), &mut audio, &mut inputs, &mut Saves)?;
            let line = emulator.hybrid_bus().vdp.scanline();
            if measure {
                summary.maximum_tick_clocks = summary
                    .maximum_tick_clocks
                    .max(clock(&emulator) - before_clock);
            }
            if measure && previous_line != line && line == 224 && game && !idle {
                summary.nonidle_vblanks.push(Lag {
                    frame,
                    pc,
                    function: symbols.at(pc).into(),
                    idle_draws,
                });
            }
            let survey = emulator.hybrid_bus_mut().timing_survey.as_mut().unwrap();
            for access in survey.accesses.drain(..) {
                if measure {
                    summary.observe(access);
                }
            }
            if effect == TickEffect::FrameRendered {
                break;
            }
        }
        if measure {
            summary.measured_frames += 1;
            summary.idle_draws_per_frame.push(idle_draws);
        }
        for (key, fork, samples) in forks.drain(..) {
            alternate_audio.samples = samples;
            compare(
                &emulator,
                &fork,
                &audio,
                &alternate_audio,
                frame,
                summary.experiments.entry(key).or_default(),
            );
        }
    }
    summary.finish(&symbols);
    fs::write(&args.output, serde_json::to_vec_pretty(&summary)?)?;
    println!(
        "revision={} frames={} measured={} groups={} ticks={} nonidle_vblanks={} experiments={}",
        summary.revision,
        summary.frames,
        summary.measured_frames,
        summary.accesses.len(),
        summary.work_ticks.len(),
        summary.nonidle_vblanks.len(),
        summary.experiments.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attribution_obeys_half_open_ranges() {
        let symbols = Symbols {
            ranges: vec![(10, 20, "first".into()), (25, 30, "second".into())],
            revision: "rev02".into(),
        };
        assert_eq!(symbols.at(9), "unattributed");
        assert_eq!(symbols.at(10), "first");
        assert_eq!(symbols.at(19), "first");
        assert_eq!(symbols.at(20), "unattributed");
        assert_eq!(symbols.at(25), "second");
        assert_eq!(symbols.at(30), "unattributed");
    }
    #[test]
    fn interrupt_and_vertical_phase_are_independent() {
        let symbols = Symbols {
            ranges: vec![(0, 8, "handler".into())],
            revision: "rev02".into(),
        };
        let mut summary = Summary::default();
        summary.observe(TimingAccess {
            clock: 0,
            scanline: 113,
            line_clock: 200,
            active_lines: 224,
            cpu: TimingCpu::M68k,
            pc: 4,
            interrupt: 4,
            kind: TimingKind::VdpControl,
            write: true,
            address: 0xC00004,
            value: 0,
        });
        summary.finish(&symbols);
        assert_eq!(
            summary.accesses["handler/68000/VdpControl/write/active/irq4/113"].accesses,
            1
        );
    }
    #[test]
    #[ignore = "requires private TJE_REV02_ROM"]
    fn passive_observation_preserves_reference() {
        let bytes = fs::read(std::env::var("TJE_REV02_ROM").unwrap()).unwrap();
        let rom = Rom::import(&bytes).unwrap();
        let mut reference = GenesisEmulator::create(
            Some(rom.into_bytes()),
            GenesisEmulatorConfig::default(),
            &mut Saves,
        );
        let mut observed = reference.partial_clone();
        observed.hybrid_bus_mut().timing_survey = Some(TimingSurvey::default());
        reference.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        observed.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        let script = InputScript::parse(include_str!("../../examples/start-game.json")).unwrap();
        let mut a = Sink::default();
        let mut b = Sink::default();
        for frame in 0..3600 {
            reference
                .hybrid_bus_mut()
                .hybrid_writes
                .as_mut()
                .unwrap()
                .clear();
            observed
                .hybrid_bus_mut()
                .hybrid_writes
                .as_mut()
                .unwrap()
                .clear();
            advance(&mut reference, script.at(frame), &mut a).unwrap();
            take_rom(&mut observed, &mut reference);
            advance(&mut observed, script.at(frame), &mut b).unwrap();
            take_rom(&mut reference, &mut observed);
            let mut result = Comparison::default();
            compare(&reference, &observed, &a, &b, frame, &mut result);
            assert_eq!(result.pixel_different_frames, 0, "pixels frame {frame}");
            assert_eq!(result.audio_different_frames, 0, "audio frame {frame}");
            assert_eq!(result.ram_different_frames, 0, "RAM frame {frame}");
            assert_eq!(
                reference.hybrid_bus().hybrid_writes,
                observed.hybrid_bus().hybrid_writes,
                "chip stream frame {frame}"
            );
            assert_eq!(clock(&reference), clock(&observed), "clock frame {frame}");
            assert_eq!(
                reference.hybrid_cpu().pc(),
                observed.hybrid_cpu().pc(),
                "PC frame {frame}"
            );
        }
    }
}
