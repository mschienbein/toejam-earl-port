use crate::StateHash;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// A snapshot contains the complete deterministic state. Restoring must also
/// invalidate host-only observers; compiled cartridge code is never copied.
pub trait Machine {
    type Snapshot;
    fn capture(&self) -> Result<Self::Snapshot>;
    fn capture_into(&self, snapshot: &mut Self::Snapshot) -> Result<()> {
        *snapshot = self.capture()?;
        Ok(())
    }
    fn snapshot_hash(snapshot: &Self::Snapshot) -> StateHash;
    fn restore(&mut self, snapshot: &Self::Snapshot) -> Result<()>;
    fn advance(&mut self, pads: [u8; 2]) -> Result<()>;
    fn state_hash(&self) -> Result<StateHash>;
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub input_delay: u8,
    pub max_rollback: u8,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            input_delay: 2,
            max_rollback: 12,
        }
    }
}
impl Config {
    pub fn validate(self) -> Result<()> {
        ensure!(self.input_delay <= 8, "input delay must be 0..=8 frames");
        ensure!(
            (2..=30).contains(&self.max_rollback),
            "rollback window must be 2..=30 frames"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Stats {
    pub frames: u64,
    pub confirmed: u64,
    pub rollbacks: u64,
    pub resimulated_frames: u64,
    pub largest_rollback: u64,
    pub stalls: u64,
}
struct Saved<S> {
    frame: u64,
    state: S,
    remote_used: u8,
}

/// Input-delay plus last-input prediction. A frame is never dropped: at the
/// rollback bound the frontend must wait, while continuing to pump transport.
/// Only the new visible frame's audio should be queued; replayed audio is not
/// emitted a second time (a corrected prediction may audibly discontinuously
/// change, but never duplicates an audio block).
pub struct Rollback<S> {
    config: Config,
    player: usize,
    next: u64,
    remote_next: u64,
    acknowledged: u64,
    local: BTreeMap<u64, u8>,
    remote: BTreeMap<u64, u8>,
    saved: VecDeque<Saved<S>>,
    recycled: Vec<S>,
    dirty: Option<u64>,
    hashes: BTreeMap<u64, StateHash>,
    remote_hashes: BTreeMap<u64, StateHash>,
    stats: Stats,
}
impl<S> Rollback<S> {
    pub fn new(player: usize, config: Config) -> Result<Self> {
        config.validate()?;
        ensure!(player < 2, "netplay requires player 0 or 1");
        let neutral: BTreeMap<_, _> = (0..u64::from(config.input_delay))
            .map(|frame| (frame, 0))
            .collect();
        Ok(Self {
            config,
            player,
            next: 0,
            remote_next: u64::from(config.input_delay),
            acknowledged: u64::from(config.input_delay),
            local: neutral.clone(),
            remote: neutral,
            saved: VecDeque::with_capacity(usize::from(config.max_rollback) + 1),
            recycled: Vec::with_capacity(usize::from(config.max_rollback) + 2),
            dirty: None,
            hashes: BTreeMap::new(),
            remote_hashes: BTreeMap::new(),
            stats: Stats::default(),
        })
    }
    pub fn stats(&self) -> Stats {
        Stats {
            frames: self.next,
            confirmed: self.confirmed(),
            ..self.stats
        }
    }
    pub fn frame(&self) -> u64 {
        self.next
    }
    pub fn confirmed(&self) -> u64 {
        self.remote_next.min(self.next)
    }
    pub fn received_through(&self) -> u64 {
        self.remote_next
    }
    pub fn inputs(&self) -> impl Iterator<Item = (u64, u8)> + '_ {
        self.local
            .range(self.acknowledged..)
            .map(|(&frame, &pad)| (frame, pad))
    }
    pub fn acknowledge(&mut self, through: u64) -> Result<()> {
        ensure!(
            through <= self.next + u64::from(self.config.input_delay),
            "peer acknowledged unsent input"
        );
        self.acknowledged = self.acknowledged.max(through);
        Ok(())
    }
    pub fn receive(&mut self, frame: u64, pad: u8) -> Result<()> {
        let floor = self
            .next
            .saturating_sub(u64::from(self.config.max_rollback) + 1);
        // Old retransmissions are normal, but inputs outside the prediction
        // horizon cannot be allowed to grow our history without bound.
        if frame < floor {
            return Ok(());
        }
        ensure!(
            frame <= self.next + 64,
            "peer input exceeds prediction horizon"
        );
        if let Some(&previous) = self.remote.get(&frame) {
            ensure!(
                previous == pad,
                "peer changed already received input at frame {frame}"
            );
            return Ok(());
        }
        if frame < self.remote_next {
            return Ok(());
        }
        if frame < self.next {
            let saved = self
                .saved
                .iter()
                .find(|saved| saved.frame == frame)
                .ok_or_else(|| {
                    anyhow::anyhow!("input arrived outside rollback window at frame {frame}")
                })?;
            if saved.remote_used != pad {
                self.dirty = Some(self.dirty.map_or(frame, |dirty| dirty.min(frame)));
            }
        }
        self.remote.insert(frame, pad);
        while self.remote.contains_key(&self.remote_next) {
            self.remote_next += 1;
        }
        Ok(())
    }
    pub fn receive_hash(&mut self, frame: u64, hash: StateHash) -> Result<()> {
        ensure!(
            frame > 0 && frame.is_multiple_of(30),
            "invalid state-hash checkpoint"
        );
        ensure!(
            frame <= self.next + 64,
            "peer hash exceeds prediction horizon"
        );
        if frame + 240 < self.next {
            return Ok(());
        }
        if let Some(previous) = self.remote_hashes.insert(frame, hash) {
            ensure!(
                previous == hash,
                "peer changed a confirmed state hash at frame {frame}"
            );
        }
        Ok(())
    }
    pub fn checkpoint(&self) -> Option<(u64, StateHash)> {
        // Transport can run between receive() and reconcile(). Never publish a
        // newly confirmed checkpoint whose prediction is still awaiting replay.
        let stable = self
            .dirty
            .map_or(self.confirmed(), |dirty| dirty.min(self.confirmed()));
        self.hashes
            .range(..=stable)
            .next_back()
            .map(|(&frame, &hash)| (frame, hash))
    }
    fn check_hashes(&self) -> Result<()> {
        for (&frame, remote) in self.remote_hashes.range(..=self.confirmed()) {
            if let Some(local) = self.hashes.get(&frame) {
                ensure!(
                    local == remote,
                    "netplay desync at confirmed frame {frame}; disconnect and check matching builds/configuration"
                );
            }
        }
        Ok(())
    }
    fn predicted_remote(&self, frame: u64) -> u8 {
        self.remote
            .range(..=frame)
            .next_back()
            .map_or(0, |(_, &pad)| pad)
    }
    fn pads(&self, frame: u64, remote: u8) -> Result<[u8; 2]> {
        let local = *self
            .local
            .get(&frame)
            .ok_or_else(|| anyhow::anyhow!("missing local input for frame {frame}"))?;
        let mut pads = [0; 2];
        pads[self.player] = local;
        pads[1 - self.player] = remote;
        Ok(pads)
    }
    fn capture_state<M: Machine<Snapshot = S>>(&mut self, machine: &M) -> Result<S> {
        if let Some(mut state) = self.recycled.pop() {
            machine.capture_into(&mut state)?;
            Ok(state)
        } else {
            machine.capture()
        }
    }
    fn record_hash<M: Machine<Snapshot = S>>(&mut self, machine: &M, frame: u64) -> Result<()> {
        let state = self.capture_state(machine)?;
        self.hashes.insert(frame, M::snapshot_hash(&state));
        self.recycled.push(state);
        Ok(())
    }
    /// Apply corrections even while the frontend is paused or prediction-bound.
    pub fn reconcile<M: Machine<Snapshot = S>>(&mut self, machine: &mut M) -> Result<()> {
        if let Some(dirty) = self.dirty.take() {
            let position = self
                .saved
                .iter()
                .position(|saved| saved.frame == dirty)
                .ok_or_else(|| anyhow::anyhow!("rollback state is no longer available"))?;
            let depth = self.next - dirty;
            machine.restore(&self.saved[position].state)?;
            while self.saved.len() > position {
                self.recycled.push(self.saved.pop_back().unwrap().state);
            }
            for frame in dirty..self.next {
                let remote = self.predicted_remote(frame);
                let state = self.capture_state(machine)?;
                self.saved.push_back(Saved {
                    frame,
                    state,
                    remote_used: remote,
                });
                machine.advance(self.pads(frame, remote)?)?;
                if (frame + 1).is_multiple_of(30) {
                    self.record_hash(machine, frame + 1)?;
                }
            }
            self.stats.rollbacks += 1;
            self.stats.resimulated_frames += depth;
            self.stats.largest_rollback = self.stats.largest_rollback.max(depth);
        }
        self.check_hashes()?;
        self.prune();
        Ok(())
    }
    /// Returns false without consuming a local input when prediction is bounded.
    pub fn advance<M: Machine<Snapshot = S>>(
        &mut self,
        machine: &mut M,
        input: u8,
    ) -> Result<bool> {
        self.reconcile(machine)?;
        if self.next >= self.remote_next + u64::from(self.config.max_rollback)
            || self.next >= self.acknowledged + 60
        {
            self.stats.stalls += 1;
            return Ok(false);
        }
        let scheduled = self.next + u64::from(self.config.input_delay);
        if self.local.insert(scheduled, input).is_some() {
            bail!("local frame was sampled twice");
        }
        let remote = self.predicted_remote(self.next);
        let state = self.capture_state(machine)?;
        self.saved.push_back(Saved {
            frame: self.next,
            state,
            remote_used: remote,
        });
        machine.advance(self.pads(self.next, remote)?)?;
        self.next += 1;
        if self.next.is_multiple_of(30) {
            self.record_hash(machine, self.next)?;
        }
        self.check_hashes()?;
        self.prune();
        Ok(true)
    }
    fn prune(&mut self) {
        // Retain the most recent remote input before the floor for prediction.
        let floor = self.confirmed().min(self.next.saturating_sub(1));
        while self.saved.front().is_some_and(|saved| saved.frame < floor) {
            self.recycled.push(self.saved.pop_front().unwrap().state);
        }
        let local_floor = floor.min(self.acknowledged);
        while self
            .local
            .first_key_value()
            .is_some_and(|(&frame, _)| frame < local_floor)
        {
            self.local.pop_first();
        }
        let remote_floor = self
            .remote
            .range(..floor)
            .next_back()
            .map_or(floor, |(&frame, _)| frame);
        while self
            .remote
            .first_key_value()
            .is_some_and(|(&frame, _)| frame < remote_floor)
        {
            self.remote.pop_first();
        }
        let hash_floor = self.next.saturating_sub(240);
        while self
            .hashes
            .first_key_value()
            .is_some_and(|(&frame, _)| frame < hash_floor)
        {
            self.hashes.pop_first();
        }
        while self
            .remote_hashes
            .first_key_value()
            .is_some_and(|(&frame, _)| frame < hash_floor)
        {
            self.remote_hashes.pop_first();
        }
    }
}
