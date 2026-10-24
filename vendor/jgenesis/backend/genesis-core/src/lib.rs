// TJE native: the peripheral-only build has no 68000 dependency.
#[cfg(feature = "emulator")]
pub mod api;
pub mod audio;
#[cfg(feature = "emulator")]
mod bus;
pub mod input;
/// TJE native: a private runtime capture, never a serialized/public game asset.
/// No cartridge bytes or 68000 execution engine are retained.
pub mod native {
    use crate::input::InputState;
    use genesis_components::{vdp::Vdp, ym2612::Ym2612};
    use ti_sn76489::Sn76489;
    use z80_emu::Z80;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
    pub enum Origin {
        Main,
        Z80,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
    pub enum DeviceWrite {
        Vdp { port: u8, value: u16 },
        Ym2612 { port: u8, value: u8 },
        Psg { value: u8 },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
    pub struct TimedWrite {
        pub clock: u64,
        pub origin: Origin,
        pub pc: u32,
        pub write: DeviceWrite,
    }

    /// TJE native: CPU-issued instruction journal, not device-commit ordering.
    /// Purpose values are stable across engines: semantic=0, prefetch=1, discarded=2.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
    pub enum CpuAccess {
        Read { address: u32, word: bool, value: u16, purpose: u8 },
        Write { address: u32, word: bool, value: u16 },
        AcknowledgeInterrupt { level: u8 },
    }

    /// TJE native: actual work-RAM commits from either processor.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
    pub struct RamWrite {
        pub address: u32,
        pub word: bool,
        pub value: u16,
    }
    impl RamWrite {
        pub const fn new(address: u32, word: bool, value: u16) -> Self {
            Self {
                address: 0xFF0000 | (address & if word { 0xFFFE } else { 0xFFFF }),
                word,
                value: if word { value } else { value & 0xFF },
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct PeripheralState {
        pub ram: Vec<u8>,
        pub audio_ram: Vec<u8>,
        pub vdp: Vdp,
        pub ym2612: Ym2612,
        pub psg: Sn76489,
        pub input: InputState,
        pub z80: Z80,
        pub z80_bank: u32,
        pub bus_request: bool,
        pub reset: bool,
        pub cycles: crate::timing::CycleCounters,
        pub timing_mode: jgenesis_common::frontend::TimingMode,
        pub region: genesis_config::GenesisRegion,
        pub audio_resampler: crate::audio::GenesisAudioResampler,
        pub config: genesis_config::GenesisEmulatorConfig,
        pub program_counter: u32,
        pub native_input_remainder: u64,
        pub open_bus: u16,
        pub data: [u32; 8],
        pub address: [u32; 8],
        pub status: u16,
        pub user_stack_pointer: u32,
        pub supervisor_stack_pointer: u32,
    }
}
pub mod timing;
// TJE timing survey: disabled unless explicitly installed by a measurement tool.
#[cfg(feature = "emulator")]
pub use timing::survey as timing_survey;

#[cfg(feature = "emulator")]
pub use api::{GenesisEmulator, GenesisError, GenesisResult};
// TJE hybrid access: expose only the bus type and opt-in observation events.
#[cfg(feature = "emulator")]
pub use bus::{
    GenesisBus, HybridAccess, HybridBusCheckpoint, HybridDmaCapture, HybridDmaStart,
    HybridExceptionBoundary, HybridWrite,
};
pub use genesis_config::{GenesisButton, GenesisInputs, GenesisJoypadState};
