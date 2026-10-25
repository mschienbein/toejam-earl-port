//! Native opt-in adapter. Creating a connection is the only path that captures
//! rollback images, opens a socket or changes the offline frame loop.
use crate::{
    backend::{GameBackend, NativeBackend},
    input::Controllers,
};
use anyhow::{Context, Result, ensure};
use std::{borrow::Cow, time::Instant};
use tje_engine_cranelift::ENGINE_VERSION;
use tje_engine_runtime::MachineSnapshot;
use tje_netplay::{
    Config, Identity, Machine, PROTOCOL_VERSION, Phase, StateHash,
    udp::{Connection, Invitation},
};

struct Adapter<'a>(&'a mut NativeBackend);
impl Machine for Adapter<'_> {
    type Snapshot = MachineSnapshot;
    fn capture(&self) -> Result<MachineSnapshot> {
        self.0.snapshot()
    }
    fn capture_into(&self, state: &mut MachineSnapshot) -> Result<()> {
        self.0.snapshot_into(state)
    }
    fn snapshot_hash(state: &MachineSnapshot) -> StateHash {
        state.state_hash()
    }
    fn restore(&mut self, state: &MachineSnapshot) -> Result<()> {
        self.0.restore(state)
    }
    fn advance(&mut self, pads: [u8; 2]) -> Result<()> {
        self.0.advance(Controllers {
            connected: [true; 2],
            buttons: pads.map(|pad| std::array::from_fn(|bit| pad & (1 << bit) != 0)),
            jam: [false; 2],
        })?;
        Ok(())
    }
    fn state_hash(&self) -> Result<StateHash> {
        Ok(self.0.snapshot()?.state_hash())
    }
}

#[derive(Default)]
pub struct Session {
    connection: Option<Connection<MachineSnapshot>>,
    started: Option<Instant>,
    player: usize,
    sample_rate: u32,
}
impl Session {
    pub fn active(&self) -> bool {
        self.connection.is_some()
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn start(
        &mut self,
        backend: &mut NativeBackend,
        action: &Action,
        sample_rate: u32,
    ) -> Result<String> {
        ensure!(
            !self.active(),
            "disconnect before creating a new netplay session"
        );
        ensure!(
            backend.profile_id() == "faithful",
            "netplay currently requires faithful simulation; disable enhanced-debug before connecting"
        );
        let (config, invitation) = match action {
            Action::Host { config, .. } => (*config, None),
            Action::Join { code, .. } => {
                let invitation = Invitation::decode(code)?;
                ensure!(
                    sample_rate == invitation.sample_rate,
                    "host uses {} Hz audio, this device uses {sample_rate} Hz; both peers need the same audio rate",
                    invitation.sample_rate
                );
                (invitation.config, Some(invitation))
            }
            Action::Disconnect => anyhow::bail!("no connection was requested"),
        };
        config.validate()?;
        // Start from local power-on, never exchange a save image or ROM bytes.
        backend.reset();
        let identity = Identity {
            protocol: PROTOCOL_VERSION,
            engine: format!("{ENGINE_VERSION}:{}", env!("CARGO_PKG_VERSION")),
            rom_sha1: backend.revision().sha1().into(),
            mode: backend.profile_id().into(),
            sample_rate,
            config,
            initial_state: backend.snapshot()?.state_hash(),
        };
        let connection = match action {
            Action::Host {
                bind, advertised, ..
            } => Connection::host(
                identity,
                bind.parse().context("host bind address must be IP:port")?,
                advertised
                    .parse()
                    .context("advertised host address must be reachable IP:port")?,
                0,
            )?,
            Action::Join { .. } => {
                let invitation = invitation.unwrap();
                let bind = if invitation.address.is_ipv6() {
                    "[::]:0"
                } else {
                    "0.0.0.0:0"
                };
                Connection::join(identity, bind.parse().unwrap(), invitation, 0)?
            }
            Action::Disconnect => unreachable!(),
        };
        let code = connection.code()?;
        self.player = usize::from(matches!(action, Action::Join { .. }));
        self.sample_rate = sample_rate;
        self.connection = Some(connection);
        self.started = Some(Instant::now());
        Ok(code)
    }
    pub fn disconnect(&mut self, backend: &mut NativeBackend) {
        self.connection = None;
        self.started = None;
        backend.reset();
    }
    pub fn poll(&mut self, backend: &mut NativeBackend) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            connection.pump(self.started.unwrap().elapsed().as_millis() as u64)?;
            connection.peer_mut().reconcile(&mut Adapter(backend))?;
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        backend: &mut NativeBackend,
        controllers: Controllers,
    ) -> Result<bool> {
        let connection = self.connection.as_mut().context("netplay is off")?;
        // Each peer uses its local P1 binding, irrespective of its remote slot.
        let input = controllers.buttons[0]
            .into_iter()
            .enumerate()
            .fold(0_u8, |pad, (bit, pressed)| pad | (u8::from(pressed) << bit));
        connection.peer_mut().advance(&mut Adapter(backend), input)
    }
    pub fn status(&self) -> Cow<'static, str> {
        let Some(connection) = &self.connection else {
            return "Off — local faithful play".into();
        };
        let stats = connection.peer().stats();
        Cow::Owned(format!(
            "{} · you are P{} (local P1 controls) · frame {} / confirmed {} · {} rollbacks / {} replayed / max {} · {} waits",
            if connection.peer().phase() == Phase::Connecting {
                "Waiting for matching peer"
            } else {
                "Direct UDP connected"
            },
            self.player + 1,
            stats.frames,
            stats.confirmed,
            stats.rollbacks,
            stats.resimulated_frames,
            stats.largest_rollback,
            stats.stalls
        ))
    }
}

pub enum Action {
    Host {
        bind: String,
        advertised: String,
        config: Config,
    },
    Join {
        code: String,
    },
    Disconnect,
}
pub struct Ui {
    bind: String,
    advertised: String,
    pub code: String,
    join_code: String,
    delay: u8,
}
impl Default for Ui {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:19791".into(),
            advertised: "127.0.0.1:19791".into(),
            code: String::new(),
            join_code: String::new(),
            delay: 2,
        }
    }
}
impl Ui {
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        active: bool,
        allowed: bool,
        status: &str,
    ) -> Option<Action> {
        ui.heading("Online two-player co-op — opt-in prototype");
        ui.label(status);
        ui.label("Direct UDP, no server or relay. Host is P1, joiner P2; both use their local P1 keyboard/gamepad. Starting or disconnecting resets to power-on. Both need the same cartridge, build and audio rate.");
        ui.small("Native codes work with native peers only; browser peers use WebRTC. Across NAT, forward the host UDP port and advertise its public IP. UDP is not encrypted: share the capability code only with a trusted peer.");
        if active {
            ui.add(
                egui::TextEdit::multiline(&mut self.code)
                    .desired_rows(4)
                    .interactive(false),
            );
            if ui.button("Copy connection code").clicked() {
                ui.ctx().copy_text(self.code.clone());
            }
            if ui.button("Disconnect and reset to local play").clicked() {
                return Some(Action::Disconnect);
            }
            ui.label("Copy/share the code, then close Settings (F1) to advance the game.");
            ui.small("Save/load, rewind, fast-forward, reset/import and recording are unavailable until disconnected. Opening settings pauses your input; the other peer waits at the rollback bound.");
            return None;
        }
        let mut action = None;
        ui.add_enabled_ui(allowed, |ui| {
            ui.label("Host bind IP:port (0.0.0.0 listens on all IPv4 interfaces)");
            ui.text_edit_singleline(&mut self.bind);
            ui.label("Reachable host IP:port to put in connection code (127.0.0.1 for two local instances)");
            ui.text_edit_singleline(&mut self.advertised);
            ui.add(egui::Slider::new(&mut self.delay, 0..=8).text("Input delay (frames)"));
            if ui.button("Host — reset and create connection code").clicked() {
                action = Some(Action::Host { bind: self.bind.clone(), advertised: self.advertised.clone(), config: Config { input_delay: self.delay, max_rollback: 12 } });
            }
            ui.separator();
            ui.label("Paste host's native connection code");
            ui.add(egui::TextEdit::multiline(&mut self.join_code).desired_rows(4));
            if ui.button("Join — reset and connect from an automatic local port").clicked() {
                action = Some(Action::Join { code: self.join_code.clone() });
            }
        });
        if !allowed {
            ui.label("Import a cartridge and stop replay/recording before connecting.");
        }
        action
    }
}

#[cfg(test)]
mod tests;
