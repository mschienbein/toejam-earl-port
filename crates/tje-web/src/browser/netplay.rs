//! The browser and native adapters use the same Rust rollback/protocol code;
//! WebRTC is solely a JS datagram transport and manual signaling UI.
use super::{CompiledCartridge, ENGINE_VERSION, WebGame, input, js_error};
use tje_engine_runtime::MachineSnapshot;
use tje_netplay::{Config, Identity, Machine, PROTOCOL_VERSION, Peer, Phase, StateHash};
use wasm_bindgen::prelude::*;

impl Machine for CompiledCartridge {
    type Snapshot = MachineSnapshot;
    fn capture(&self) -> anyhow::Result<MachineSnapshot> {
        #[cfg(feature = "enhanced-mode")]
        return Ok(self.enhanced.snapshot(&self.engine)?);
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.snapshot()?)
    }
    fn restore(&mut self, snapshot: &MachineSnapshot) -> anyhow::Result<()> {
        #[cfg(feature = "enhanced-mode")]
        self.enhanced.restore(&mut self.engine, snapshot)?;
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine.restore(snapshot)?;
        Ok(())
    }
    fn capture_into(&self, state: &mut MachineSnapshot) -> anyhow::Result<()> {
        #[cfg(feature = "enhanced-mode")]
        return Ok(self.enhanced.snapshot_into(&self.engine, state)?);
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.snapshot_into(state)?)
    }
    fn snapshot_hash(state: &MachineSnapshot) -> StateHash {
        state.state_hash()
    }
    fn advance(&mut self, pads: [u8; 2]) -> anyhow::Result<()> {
        #[cfg(feature = "enhanced-mode")]
        self.enhanced
            .play_frame(&mut self.engine, input::inputs(pads[0], pads[1], true))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine
            .play_frame(input::inputs(pads[0], pads[1], true))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(())
    }
    fn state_hash(&self) -> anyhow::Result<StateHash> {
        Ok(self.capture()?.state_hash())
    }
}

#[wasm_bindgen]
impl WebGame {
    /// Called after the local audio context has established its sample rate.
    /// No ROM or machine image crosses into the data-channel protocol.
    pub fn netplay_start(
        &mut self,
        player: u8,
        token: &[u8],
        delay: u8,
        window: u8,
        now_ms: f64,
    ) -> Result<(), JsValue> {
        if self.netplay.is_some() {
            return Err(js_error("disconnect the current netplay session first"));
        }
        if self
            .cartridge
            .as_ref()
            .is_some_and(|cartridge| cartridge.profile_id() != "faithful")
        {
            return Err(js_error(
                "netplay currently requires faithful simulation; disable enhanced-debug before connecting",
            ));
        }
        let token: [u8; 16] = token
            .try_into()
            .map_err(|_| js_error("invalid connection capability"))?;
        let config = Config {
            input_delay: delay,
            max_rollback: window,
        };
        config.validate().map_err(js_error)?;
        if player > 1 {
            return Err(js_error("netplay requires player 0 or 1"));
        }
        self.rewind = None;
        self.reset()?;
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        let identity = Identity {
            protocol: PROTOCOL_VERSION,
            engine: format!("{ENGINE_VERSION}:{}", env!("CARGO_PKG_VERSION")),
            rom_sha1: cartridge.revision.sha1().into(),
            mode: cartridge.profile_id().into(),
            sample_rate: self.sample_rate,
            config,
            initial_state: cartridge.capture().map_err(js_error)?.state_hash(),
        };
        self.netplay = Some(Peer::new(identity, token, player, now_ms as u64).map_err(js_error)?);
        Ok(())
    }
    pub fn netplay_packet(&self) -> Result<Vec<u8>, JsValue> {
        self.netplay
            .as_ref()
            .ok_or_else(|| js_error("netplay is off"))?
            .packet()
            .map_err(js_error)
    }
    pub fn netplay_receive(&mut self, bytes: &[u8], now_ms: f64) -> Result<(), JsValue> {
        self.netplay
            .as_mut()
            .ok_or_else(|| js_error("netplay is off"))?
            .receive(bytes, now_ms as u64)
            .map_err(js_error)?;
        Ok(())
    }
    pub fn netplay_poll(&mut self, now_ms: f64) -> Result<(), JsValue> {
        if let Some(peer) = &mut self.netplay {
            peer.check_timeout(now_ms as u64).map_err(js_error)?;
            let cartridge = self
                .cartridge
                .as_mut()
                .ok_or_else(|| js_error("cartridge was unloaded during netplay"))?;
            peer.reconcile(cartridge).map_err(js_error)?;
        }
        Ok(())
    }
    pub fn netplay_step(&mut self, pad: u8) -> Result<bool, JsValue> {
        let peer = self
            .netplay
            .as_mut()
            .ok_or_else(|| js_error("netplay is off"))?;
        let cartridge = self
            .cartridge
            .as_mut()
            .ok_or_else(|| js_error("cartridge was unloaded during netplay"))?;
        if !peer.advance(cartridge, pad).map_err(js_error)? {
            return Ok(false);
        }
        let samples = cartridge.engine.devices().audio_samples();
        self.audio.clear();
        self.audio.reserve(samples.len() * 2);
        for &[left, right] in samples {
            self.audio.push(left as f32);
            self.audio.push(right as f32);
        }
        Ok(true)
    }
    pub fn netplay_stop(&mut self) -> Result<(), JsValue> {
        self.netplay = None;
        self.reset()
    }
    pub fn netplay_status(&self) -> Result<String, JsValue> {
        let peer = self
            .netplay
            .as_ref()
            .ok_or_else(|| js_error("netplay is off"))?;
        serde_json::to_string(&serde_json::json!({
            "phase": if peer.phase() == Phase::Running { "running" } else { "connecting" },
            "stats": peer.stats(),
        }))
        .map_err(js_error)
    }
}
