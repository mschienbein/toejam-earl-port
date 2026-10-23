//! Opt-in instruction-entry coverage and completed logic-tick observations.
//!
//! This module never changes CPU timing or memory. Traces are private runtime
//! observations, not serializable save states, and must not be committed.

use crate::{Host, HostError, Result};

#[derive(Debug)]
pub struct EntryObservation {
    pub address: u32,
    pub frame: u64,
    /// The first four MCC68K 32-bit stack slots, observed before the prologue.
    /// An observation is not a claim that the function declares four arguments.
    pub stack_slots: [u32; 4],
    /// Bytes at the caller-configured canonical work-RAM addresses.
    pub ram_samples: Vec<u8>,
}

#[derive(Debug)]
pub struct LogicTick {
    pub ordinal: u64,
    pub frame: u64,
    pub pc: u32,
    pub stack_pointer: u32,
    pub last_entry: Option<u32>,
    /// True when a physical VBlank crossed while this logic iteration was busy.
    pub lagged: bool,
    pub ram: Box<[u8]>,
}

/// All returned buffers are bounded by one call to `run_frame`, except the fixed
/// entry-count array. Call `take_session_observations` after each video frame.
#[derive(Default, Debug)]
pub struct FrameObservations {
    pub ticks: Vec<LogicTick>,
    pub entries: Vec<EntryObservation>,
}

pub(crate) struct SessionTrace {
    slots: Vec<u16>,
    counts: Vec<u64>,
    watched: Vec<bool>,
    sample_addresses: Vec<u32>,
    capture_ticks: bool,
    observations: FrameObservations,
    ordinal: u64,
    active: bool,
    lagged: bool,
    last_entry: Option<u32>,
}

impl Host {
    /// Track exactly the supplied entry addresses. Counts reflect executable PC
    /// entry, including tail transfers/fragments; they are not inferred calls.
    pub fn enable_session_trace(
        &mut self,
        entries: &[u32],
        watched: &[u32],
        samples: &[u32],
        capture_ticks: bool,
    ) -> Result<()> {
        if self.frame_count() != 0 {
            return Err(HostError::Metadata(
                "enable session trace before power-on playback".into(),
            ));
        }
        if entries.len() >= u16::MAX as usize
            || entries.iter().any(|pc| pc & 1 != 0 || *pc >= 0x100000)
        {
            return Err(HostError::Metadata(
                "invalid session trace entry addresses".into(),
            ));
        }
        let mut slots = vec![0_u16; entries.iter().copied().max().unwrap_or(0) as usize / 2 + 1];
        for (index, &pc) in entries.iter().enumerate() {
            if slots[pc as usize / 2] != 0 {
                return Err(HostError::Metadata(
                    "duplicate session trace entry address".into(),
                ));
            }
            slots[pc as usize / 2] = index as u16 + 1;
        }
        if watched.iter().any(|pc| !entries.contains(pc)) {
            return Err(HostError::Metadata(
                "watched address is not a tracked function entry".into(),
            ));
        }
        if samples
            .iter()
            .any(|address| !(0xFF0000..=0xFFFFFF).contains(address))
        {
            return Err(HostError::Metadata(
                "trace samples must be canonical work RAM".into(),
            ));
        }
        self.session_trace = Some(SessionTrace {
            slots,
            counts: vec![0; entries.len()],
            watched: entries.iter().map(|pc| watched.contains(pc)).collect(),
            sample_addresses: samples.to_vec(),
            capture_ticks,
            observations: FrameObservations::default(),
            ordinal: 0,
            active: false,
            lagged: false,
            last_entry: None,
        });
        Ok(())
    }

    pub fn session_entry_counts(&self) -> Option<&[u64]> {
        self.session_trace
            .as_ref()
            .map(|trace| trace.counts.as_slice())
    }

    pub fn take_session_observations(&mut self) -> FrameObservations {
        self.session_trace
            .as_mut()
            .map_or_else(FrameObservations::default, |trace| {
                std::mem::take(&mut trace.observations)
            })
    }

    pub(crate) fn trace_instruction(&mut self, pc: u32) {
        let Some(trace) = &mut self.session_trace else {
            return;
        };
        if let Some(&slot) = trace.slots.get(pc as usize / 2)
            && slot != 0
        {
            let index = usize::from(slot - 1);
            trace.counts[index] += 1;
            trace.last_entry = Some(pc);
            if trace.watched[index] {
                let sp = self.emulator.hybrid_cpu().stack_pointer();
                let (ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                let stack_slots = std::array::from_fn(|index| {
                    let address = sp.wrapping_add(4 + 4 * index as u32);
                    let high = ram[((address & 0xFFFF) / 2) as usize];
                    let low = ram[(((address + 2) & 0xFFFF) / 2) as usize];
                    (u32::from(high) << 16) | u32::from(low)
                });
                let ram_samples = trace
                    .sample_addresses
                    .iter()
                    .map(|address| {
                        let word = ram[((address & 0xFFFF) / 2) as usize];
                        word.to_be_bytes()[(address & 1) as usize]
                    })
                    .collect();
                trace.observations.entries.push(EntryObservation {
                    address: pc,
                    frame: self.frame,
                    stack_slots,
                    ram_samples,
                });
            }
        }
        if pc == self.symbols.main_loop {
            trace.active = true;
            trace.lagged = false;
        } else if pc == self.symbols.main_wait && trace.active {
            if trace.capture_ticks {
                let sp = self.emulator.hybrid_cpu().stack_pointer();
                let (ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                let ram = ram
                    .iter()
                    .flat_map(|word| word.to_be_bytes())
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                trace.observations.ticks.push(LogicTick {
                    ordinal: trace.ordinal,
                    frame: self.frame,
                    pc,
                    stack_pointer: sp,
                    last_entry: trace.last_entry,
                    lagged: trace.lagged,
                    ram,
                });
            }
            trace.ordinal += 1;
            trace.active = false;
        }
    }

    pub(crate) fn trace_lag(&mut self) {
        if let Some(trace) = &mut self.session_trace {
            trace.lagged = true;
        }
    }
}
