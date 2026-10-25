//! Private controller-only search. Snapshots never leave memory; reports contain counts/timing only.
use anyhow::{Context, Result, ensure};
use bincode::Decode;
use clap::Parser;
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use genesis_core::{
    GenesisEmulator,
    timing_survey::{TimingAccess, TimingCpu, TimingKind, TimingSurvey},
};
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, PartialClone, RenderFrameOptions,
    Renderer, SaveWriter, TickEffect,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    fs, io,
    path::PathBuf,
};
use tje_rom::{Revision, Rom};
use tje_session::{CONNECTED, Session, Start};

mod fixtures;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long, required = true, num_args = 1..)]
    session: Vec<PathBuf>,
    #[arg(long)]
    report: PathBuf,
    #[arg(long)]
    output_dir: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    /// Checkpoints in the baseline power-on session, half-open frame range.
    #[arg(long, default_value_t = 0)]
    sweep_start: usize,
    #[arg(long, default_value_t = 0)]
    sweep_end: usize,
    #[arg(long, default_value_t = 16)]
    stride: usize,
    #[arg(long, default_value_t = 24)]
    horizon: usize,
    #[arg(long, default_value_t = 0)]
    phase_max: usize,
    /// Compare the passive observer with an independent, unobserved original each frame.
    #[arg(long)]
    neutrality: bool,
    /// Explicitly non-natural, one-byte transient fixture; never counted as corpus coverage.
    #[arg(long, value_parser = ["pcm", "psg"])]
    fixture: Option<String>,
    #[arg(long, default_value_t = 1)]
    fixture_call: u64,
    /// Replay the exact selector/binding from a prior labelled fixture report.
    #[arg(long)]
    fixture_recipe: Option<PathBuf>,
}
#[derive(Default)]
struct Sink;
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
    fn push_sample(&mut self, _: f64, _: f64) -> Result<(), Self::Err> {
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
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (i, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << i) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
#[derive(Clone, Serialize)]
struct Event {
    frame: usize,
    clock: u64,
    cpu: &'static str,
    pc: u32,
    address: u32,
    write: bool,
    value: u16,
}
#[derive(Clone, Serialize)]
struct Hit {
    frame: usize,
    pc: u32,
    clock: u64,
    context: Vec<Event>,
}
#[derive(Clone, Default, Serialize)]
struct Span {
    count: u64,
    min: Option<u64>,
    max: u64,
    total: u64,
}
impl Span {
    fn add(&mut self, n: u64) {
        self.count += 1;
        self.min = Some(self.min.map_or(n, |v| v.min(n)));
        self.max = self.max.max(n);
        self.total += n;
    }
}
#[derive(Clone, Default, Serialize)]
struct Stats {
    frames: u64,
    variants: u64,
    checkpoints: u64,
    executed_pcs: BTreeMap<u32, u64>,
    mailbox_reads: BTreeMap<u32, BTreeMap<u16, u64>>,
    command_writes: BTreeMap<u16, u64>,
    command_to_lock_start: Span,
    command_to_owner_read: BTreeMap<u32, Span>,
    lock_residence: Span,
    lock_free_to_owner_read: BTreeMap<u32, Span>,
    lock_free_closest: BTreeMap<u32, Event>,
    lock_busreqs: u64,
    service_spacing: Span,
    service_duration: Span,
    release_to_service: Span,
    same_video_frame_services: u64,
    hits: Vec<Hit>,
    #[serde(skip)]
    busy_start: Option<u64>,
    #[serde(skip)]
    busy_end: Option<u64>,
    #[serde(skip)]
    command: Option<u64>,
    #[serde(skip)]
    last_service: Option<u64>,
    #[serde(skip)]
    last_service_frame: Option<usize>,
    #[serde(skip)]
    service_start: Option<u64>,
    #[serde(skip)]
    last_release: Option<u64>,
    #[serde(skip)]
    history: VecDeque<Event>,
}
impl Stats {
    fn access(&mut self, access: TimingAccess, frame: usize) {
        let z80 = access.cpu == TimingCpu::Z80;
        let lock = access.kind == TimingKind::Z80Ram
            && access.address == if z80 { 0x41 } else { 0xa00041 };
        let command =
            access.kind == TimingKind::Z80Ram && !z80 && access.address == 0xa00040 && access.write;
        let bus = access.kind == TimingKind::Z80BusRequest;
        if !lock && !command && !bus {
            return;
        }
        let event = Event {
            frame,
            clock: access.clock,
            cpu: if z80 { "z80" } else { "m68k" },
            pc: access.pc,
            address: access.address,
            write: access.write,
            value: access.value,
        };
        if command {
            *self.command_writes.entry(access.value).or_default() += 1;
            self.command = (access.value == 2).then_some(access.clock);
        }
        if bus && access.write && access.value & 0x100 != 0 && self.busy_start.is_some() {
            self.lock_busreqs += 1;
        }
        if bus && access.write && access.value == 0 {
            self.last_release = Some(access.clock);
        }
        if lock && z80 && access.write {
            if access.value == 1 {
                self.busy_start = Some(access.clock);
                if let Some(command) = self.command {
                    self.command_to_lock_start
                        .add(access.clock.saturating_sub(command));
                }
            } else if let Some(start) = self.busy_start.take() {
                self.lock_residence.add(access.clock - start);
                self.busy_end = Some(access.clock);
            }
        }
        if lock && !z80 && !access.write {
            *self
                .mailbox_reads
                .entry(access.pc)
                .or_default()
                .entry(access.value)
                .or_default() += 1;
            if let Some(command) = self.command {
                self.command_to_owner_read
                    .entry(access.pc)
                    .or_default()
                    .add(access.clock.saturating_sub(command));
            }
            if let Some(end) = self.busy_end {
                let span = self.lock_free_to_owner_read.entry(access.pc).or_default();
                let elapsed = access.clock.saturating_sub(end);
                if span.min.is_none_or(|n| elapsed < n) {
                    self.lock_free_closest.insert(access.pc, event.clone());
                }
                span.add(elapsed);
            }
        }
        self.history.push_back(event);
        if self.history.len() > 96 {
            self.history.pop_front();
        }
    }
    fn pc(&mut self, pc: u32, frame: usize, clock: u64, pcs: &[u32; 7]) {
        if !pcs.contains(&pc) {
            return;
        }
        *self.executed_pcs.entry(pc).or_default() += 1;
        if pc == pcs[0] || pc == pcs[1] {
            self.hits.push(Hit {
                frame,
                pc,
                clock,
                context: self.history.iter().cloned().collect(),
            });
        }
        if pc == pcs[5] {
            if let Some(previous) = self.last_service {
                self.service_spacing.add(clock - previous);
            }
            if let Some(release) = self.last_release {
                self.release_to_service.add(clock - release);
            }
            self.last_service = Some(clock);
            self.service_start = Some(clock);
            if self.last_service_frame == Some(frame) {
                self.same_video_frame_services += 1;
            }
            self.last_service_frame = Some(frame);
        }
        if pc == pcs[6]
            && let Some(start) = self.service_start.take()
        {
            self.service_duration.add(clock - start);
        }
    }
}
fn step_frame(
    machine: &mut GenesisEmulator,
    pads: [u16; 2],
    frame: usize,
    stats: &mut Stats,
    pcs: &[u32; 7],
) -> Result<()> {
    let mut pads = Inputs(inputs(pads));
    loop {
        let cpu = machine.hybrid_cpu();
        if machine.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
            && cpu.hybrid_instruction_ready(machine.hybrid_bus())
        {
            stats.pc(
                cpu.pc(),
                frame,
                machine.hybrid_bus().cycles.m68k_mclk_cycles,
                pcs,
            );
        }
        let effect = machine.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves)?;
        for access in machine
            .hybrid_bus_mut()
            .timing_survey
            .as_mut()
            .unwrap()
            .accesses
            .drain(..)
        {
            stats.access(access, frame);
        }
        if effect == TickEffect::FrameRendered {
            stats.frames += 1;
            return Ok(());
        }
    }
}
fn plain_frame(machine: &mut GenesisEmulator, pads: [u16; 2]) -> Result<()> {
    let mut pads = Inputs(inputs(pads));
    loop {
        if machine.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves)? == TickEffect::FrameRendered {
            return Ok(());
        }
    }
}
fn take_rom(to: &mut GenesisEmulator, from: &mut GenesisEmulator) {
    to.hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .take_rom_from(from.hybrid_bus_mut().cartridge.as_mut().unwrap());
}
fn save_hit(
    args: &Args,
    revision: Revision,
    baseline: &[[u16; 2]],
    checkpoint: usize,
    variant: &[[u16; 2]],
    stats: &Stats,
    tag: &str,
) -> Result<()> {
    let Some(directory) = &args.output_dir else {
        return Ok(());
    };
    fs::create_dir_all(directory)?;
    for hit in &stats.hits {
        let suffix = format!("{tag}-f{}-pc{:06x}", hit.frame, hit.pc);
        let mut session = Session::new(
            revision,
            format!(
                "Controller-only natural audio retry search hit; {suffix}; no recipes, state edits, timing changes or injected requests"
            ),
        );
        for &pads in baseline
            .iter()
            .take(checkpoint)
            .chain(variant.iter().take(hit.frame - checkpoint + 1))
        {
            session.record(pads)?;
        }
        fs::write(
            directory.join(format!("{suffix}.tje.json")),
            session.to_json()?,
        )?;
        fs::write(
            directory.join(format!("{suffix}.evidence.json")),
            serde_json::to_vec_pretty(hit)?,
        )?;
    }
    Ok(())
}
#[derive(Serialize)]
struct Report {
    revision: String,
    source: String,
    baseline: Stats,
    sweep: Stats,
    sweep_parameters: serde_json::Value,
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.stride > 0 && args.horizon > 0,
        "positive stride/horizon required"
    );
    let rom_bytes = fs::read(&args.rom)?;
    let rom = Rom::import(&rom_bytes)?;
    let revision = rom.revision();
    let pcs = match revision {
        Revision::Rev02 => [
            0x3f61c, 0x3f79c, 0x3f62a, 0x3f7aa, 0x3f6c6, 0x3ed6e, 0x3f068,
        ],
        Revision::Rev00 => [
            0x3f5ac, 0x3f72c, 0x3f5ba, 0x3f73a, 0x3f656, 0x3ecfe, 0x3eff8,
        ],
    };
    if args.fixture.is_some() || args.fixture_recipe.is_some() {
        return fixtures::run(&args, &rom, &pcs);
    }
    let mut reports = Vec::new();
    for source in &args.session {
        let session = Session::from_json(&fs::read_to_string(source)?)?;
        ensure!(
            matches!(session.start, Start::PowerOn),
            "input-only power-on session required: {}",
            source.display()
        );
        ensure!(
            Revision::from(session.revision) == revision,
            "session revision mismatch"
        );
        let baseline: Vec<_> = session
            .inputs()
            .take(args.limit.unwrap_or(usize::MAX))
            .collect();
        let mut machine = GenesisEmulator::create(
            Some(rom.bytes().to_vec()),
            GenesisEmulatorConfig::default(),
            &mut Saves,
        );
        machine.hybrid_bus_mut().timing_survey = Some(TimingSurvey::default());
        let mut stats = Stats::default();
        let mut plain = args.neutrality.then(|| {
            GenesisEmulator::create(
                Some(rom.bytes().to_vec()),
                GenesisEmulatorConfig::default(),
                &mut Saves,
            )
        });
        let mut sweep = Stats::default();
        // Both players, simultaneous opposite roles, action+graphics and held/pulsed inputs.
        let actions: [u16; 16] = [0, 1, 2, 4, 8, 16, 32, 64, 128, 48, 80, 96, 112, 17, 24, 192];
        for (frame, &pads) in baseline.iter().enumerate() {
            if (args.sweep_start..args.sweep_end).contains(&frame)
                && (frame - args.sweep_start).is_multiple_of(args.stride)
            {
                sweep.checkpoints += 1;
                for phase in 0..=args.phase_max {
                    for (action_index, &action) in actions.iter().enumerate() {
                        for pattern in 0..4 {
                            let mut fork = machine.partial_clone();
                            take_rom(&mut fork, &mut machine);
                            let mut local = Stats {
                                busy_start: stats.busy_start,
                                busy_end: stats.busy_end,
                                command: stats.command,
                                history: stats.history.clone(),
                                last_service: stats.last_service,
                                last_service_frame: stats.last_service_frame,
                                last_release: stats.last_release,
                                service_start: stats.service_start,
                                ..Stats::default()
                            };
                            let mut variant = Vec::with_capacity(args.horizon + phase);
                            for offset in 0..args.horizon + phase {
                                let mut current =
                                    baseline.get(frame + offset).copied().unwrap_or(pads);
                                if offset >= phase {
                                    let age = offset - phase;
                                    let pulse = match pattern {
                                        0 => true,
                                        1 => age < 2,
                                        2 => age % 4 < 2,
                                        _ => age % 8 < 4,
                                    };
                                    let active = if pulse { action } else { 0 };
                                    current[0] = CONNECTED | active;
                                    if current[1] & CONNECTED != 0 {
                                        current[1] = CONNECTED
                                            | actions[(action_index + 5 + pattern) % actions.len()];
                                    }
                                }
                                variant.push(current);
                                step_frame(&mut fork, current, frame + offset, &mut local, &pcs)?;
                                if !local.hits.is_empty() {
                                    break;
                                }
                            }
                            take_rom(&mut machine, &mut fork);
                            local.variants = 1;
                            let tag = format!(
                                "{}-c{frame}-phase{phase}-a{action}-p{pattern}",
                                source.file_stem().unwrap().to_string_lossy()
                            );
                            if !local.hits.is_empty() {
                                save_hit(
                                    &args, revision, &baseline, frame, &variant, &local, &tag,
                                )?;
                                eprintln!("HIT {tag} {}", serde_json::to_string(&local.hits)?);
                            }
                            merge(&mut sweep, local);
                        }
                    }
                }
                if sweep.checkpoints.is_multiple_of(20) {
                    eprintln!(
                        "sweep baseline={frame} variants={} frames={} retry_hits={}",
                        sweep.variants,
                        sweep.frames,
                        sweep.hits.len()
                    );
                }
            }
            let prior = stats.hits.len();
            step_frame(&mut machine, pads, frame, &mut stats, &pcs)?;
            if let Some(plain) = &mut plain {
                plain_frame(plain, pads)?;
                let survey = machine.hybrid_bus_mut().timing_survey.take();
                let observed = bincode::encode_to_vec(&machine, bincode::config::standard())?;
                let unobserved = bincode::encode_to_vec(&*plain, bincode::config::standard())?;
                machine.hybrid_bus_mut().timing_survey = survey;
                ensure!(
                    observed == unobserved,
                    "passive observation changed original machine at frame {frame}"
                );
            }
            if stats.hits.len() != prior {
                save_hit(
                    &args,
                    revision,
                    &baseline,
                    0,
                    &baseline,
                    &Stats {
                        hits: stats.hits[prior..].to_vec(),
                        ..Stats::default()
                    },
                    source.file_stem().unwrap().to_str().context("UTF-8 stem")?,
                )?;
            }
            if frame.is_multiple_of(10000) {
                eprintln!(
                    "baseline {} frame={frame} locks={} hits={}",
                    source.display(),
                    stats.lock_residence.count,
                    stats.hits.len()
                );
            }
        }
        reports.push(Report { revision: format!("{revision:?}"), source: source.display().to_string(), baseline: stats, sweep, sweep_parameters: serde_json::json!({"start":args.sweep_start,"end":args.sweep_end,"stride":args.stride,"horizon":args.horizon,"phase_max":args.phase_max,"observer_neutrality":args.neutrality,"actions":actions,"patterns":["hold","two-frame-edge","2-on-2-off","4-on-4-off"],"connected_player2":"independent action offset 5+pattern"}) });
        fs::write(&args.report, serde_json::to_vec_pretty(&reports)?)?;
    }
    Ok(())
}
fn merge_span(to: &mut Span, from: Span) {
    to.count += from.count;
    to.total += from.total;
    to.max = to.max.max(from.max);
    if let Some(n) = from.min {
        to.min = Some(to.min.map_or(n, |m| m.min(n)));
    }
}
fn merge(to: &mut Stats, from: Stats) {
    to.frames += from.frames;
    to.variants += from.variants;
    for (pc, count) in from.executed_pcs {
        *to.executed_pcs.entry(pc).or_default() += count;
    }
    for (pc, values) in from.mailbox_reads {
        for (value, count) in values {
            *to.mailbox_reads
                .entry(pc)
                .or_default()
                .entry(value)
                .or_default() += count;
        }
    }
    for (value, count) in from.command_writes {
        *to.command_writes.entry(value).or_default() += count;
    }
    merge_span(&mut to.command_to_lock_start, from.command_to_lock_start);
    merge_span(&mut to.lock_residence, from.lock_residence);
    for (pc, span) in from.command_to_owner_read {
        merge_span(to.command_to_owner_read.entry(pc).or_default(), span);
    }
    for (pc, span) in from.lock_free_to_owner_read {
        if span.min < to.lock_free_to_owner_read.get(&pc).and_then(|v| v.min)
            || !to.lock_free_to_owner_read.contains_key(&pc)
        {
            if let Some(event) = from.lock_free_closest.get(&pc) {
                to.lock_free_closest.insert(pc, event.clone());
            }
        }
        merge_span(to.lock_free_to_owner_read.entry(pc).or_default(), span);
    }
    to.lock_busreqs += from.lock_busreqs;
    merge_span(&mut to.service_spacing, from.service_spacing);
    merge_span(&mut to.service_duration, from.service_duration);
    merge_span(&mut to.release_to_service, from.release_to_service);
    to.same_video_frame_services += from.same_video_frame_services;
    to.hits.extend(from.hits);
}
