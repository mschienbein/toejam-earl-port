//! One private analysis asset, never the live machine's cartridge.
use crate::{Host, HostError, Result};
use genesis_components::cartridge::Cartridge;
use genesis_config::GenesisEmulatorConfig;
use genesis_core::GenesisEmulator;

pub(super) struct AnalysisRom(Owner);

enum Owner {
    Depot(Cartridge),
    // Created only after with_fork_rom has loaned the depot into this analysis
    // Host. It cannot be inferred from Mode or copied from an emulator snapshot.
    Borrowed,
}

impl AnalysisRom {
    pub(super) fn new(bytes: &[u8], config: &GenesisEmulatorConfig) -> Self {
        Self(Owner::Depot(Cartridge::new(
            bytes.to_vec(),
            None,
            config.forced_region,
            &config.cheat_codes,
        )))
    }
}

mod sealed {
    pub trait Target {}
    impl Target for genesis_core::GenesisEmulator {}
    impl Target for crate::Host {}
}

/// Only a ROM-less analysis emulator or private analysis Host may receive a loan.
/// The sealed target distinction lets one cleanup boundary borrow a whole Host
/// (including its nested check forks), without borrowing its live parent.
pub(super) trait Target: sealed::Target {
    fn emulator(&mut self) -> &mut GenesisEmulator;
    fn host(&mut self) -> Option<&mut Host> {
        None
    }
}
impl Target for GenesisEmulator {
    fn emulator(&mut self) -> &mut GenesisEmulator {
        self
    }
}
impl Target for Host {
    fn emulator(&mut self) -> &mut GenesisEmulator {
        &mut self.emulator
    }
    fn host(&mut self) -> Option<&mut Host> {
        Some(self)
    }
}

impl Host {
    fn analysis_cartridge(&mut self) -> Result<&mut Cartridge> {
        let Some(analysis_rom) = &mut self.analysis_rom else {
            return Err(HostError::Metadata(
                "Host has no opted-in analysis ROM asset".into(),
            ));
        };
        match &mut analysis_rom.0 {
            Owner::Depot(cartridge) => Ok(cartridge),
            Owner::Borrowed => Ok(self
                .emulator
                .hybrid_bus_mut()
                .cartridge
                .as_mut()
                .expect("loaned analysis cartridge")),
        }
    }

    /// Loan the one cached asset only to analysis targets. The live emulator is
    /// never mutably borrowed on the Depot path. Nested private Hosts reuse the
    /// same allocation; neither Result errors nor unwinds restore game snapshots.
    pub(super) fn with_fork_rom<T, R: Target>(
        &mut self,
        target: &mut R,
        execute: impl FnOnce(&mut R) -> Result<T>,
    ) -> Result<T> {
        if let Some(host) = target.host() {
            if host.revision != self.revision || host.registry.revision() != self.revision {
                return Err(HostError::Metadata(
                    "analysis Host targets a different ROM revision".into(),
                ));
            }
            if host.analysis_rom.is_some() {
                return Err(HostError::Metadata(
                    "analysis Host already owns a ROM asset".into(),
                ));
            }
        }
        let destination = target
            .emulator()
            .hybrid_bus()
            .cartridge
            .as_ref()
            .ok_or_else(|| HostError::Metadata("analysis fork has no cartridge".into()))?;
        if destination.rom_len() != 0 {
            return Err(HostError::Metadata(
                "analysis fork already owns a ROM asset".into(),
            ));
        }
        let source = self.analysis_cartridge()?;
        if source.rom_len() == 0 {
            return Err(HostError::Metadata(
                "analysis ROM asset is already loaned".into(),
            ));
        }
        target
            .emulator()
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("validated analysis cartridge")
            .take_rom_from(source);
        if let Some(host) = target.host() {
            host.analysis_rom = Some(AnalysisRom(Owner::Borrowed));
        }
        // The caller retains the target outside this catch, so an unrelated
        // panic cannot drop the cartridge before the exact asset is reclaimed.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(target)));
        self.analysis_cartridge()
            .expect("retained analysis asset owner")
            .take_rom_from(
                target
                    .emulator()
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("retained analysis cartridge"),
            );
        if let Some(host) = target.host() {
            host.analysis_rom = None;
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
