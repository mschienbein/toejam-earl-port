use bincode::{Decode, Encode};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::{
    AudioOutput, Color, FrameSize, InputPoller, RenderFrameOptions, Renderer, SaveWriter,
};
use std::{convert::Infallible, io};

#[derive(Default)]
pub(crate) struct HeadlessOutput;
impl Renderer for HeadlessOutput {
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
pub(crate) struct HeadlessAudio {
    pub samples: Vec<[f64; 2]>,
}

impl Default for HeadlessAudio {
    fn default() -> Self {
        Self {
            samples: Vec::with_capacity(1024),
        }
    }
}

impl AudioOutput for HeadlessAudio {
    type Err = Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Self::Err> {
        self.samples.push([left, right]);
        Ok(())
    }
}

pub(crate) struct Inputs(pub GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}

/// ToeJam & Earl has no cartridge save RAM. Fail rather than silently discard a
/// persistence request if this host is accidentally reused with another ROM.
#[derive(Default)]
pub(crate) struct NoCartridgeSaves;
impl SaveWriter for NoCartridgeSaves {
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
    fn persist_serialized<E: Encode>(&mut self, _: &str, _: E) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
