//! The original relative schedule expressed as bounded typed service calls.
use crate::{Admission, Error, Phase, PublicationStamp, Result, session::Session};
use tje_four_player::input::InputFrame;

/// Saved ownership of the original caller's finite phase traversal. This is
/// independent of presentation calls and of the hardware frame in which a
/// later original caller acknowledgement happens to execute.
#[derive(Debug, Clone, Default, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct State {
    pub pending_input: Option<InputFrame>,
    pub phase: Option<Phase>,
    pub epoch: u64,
    pub next_ack: u8,
    pub initial_animations: bool,
    pub video_frames: u64,
    pub last_video_hardware: Option<u64>,
    pub normal_music: u8,
    pub scene_stage: u8,
    pub speech: [crate::publication::PublishedSpeech; 4],
}
impl State {
    pub fn validate(&self, clock: &crate::PhaseClock) -> Result<()> {
        if self.normal_music > 31 || self.scene_stage > 4 {
            return Err(Error::State("kernel audio/scene continuation"));
        }
        for speech in &self.speech {
            if usize::from(speech.length) > speech.text.len()
                || speech.text[usize::from(speech.length)..]
                    .iter()
                    .any(|&byte| byte != 0)
            {
                return Err(Error::State("saved mode speech extent"));
            }
        }
        match self.phase {
            None if self.epoch != 0 || self.next_ack != 0 => {
                return Err(Error::State("phase acknowledgement has no owner"));
            }
            Some(phase)
                if self.epoch != clock.epoch()
                    || phase.next() != clock.next()
                    || self.next_ack > Self::acknowledgements(phase) =>
            {
                return Err(Error::State("phase acknowledgement owner/cursor"));
            }
            _ => {}
        }
        if let Some(input) = self.pending_input {
            input
                .validate()
                .map_err(|_| Error::State("queued enhanced input"))?;
            if clock.input().is_some_and(|phase| phase.frame > input.frame) {
                return Err(Error::State(
                    "phase input is newer than the queued hardware input",
                ));
            }
            if self
                .last_video_hardware
                .is_some_and(|last| last > input.frame)
            {
                return Err(Error::State("video admission is newer than queued input"));
            }
        } else if self.last_video_hardware.is_some() {
            return Err(Error::State("video admission has no input owner"));
        }
        Ok(())
    }
    pub const fn acknowledgements(phase: Phase) -> u8 {
        if phase.publishes() { 3 } else { 9 }
    }
    pub fn complete(&self) -> bool {
        self.phase
            .is_none_or(|phase| self.next_ack == Self::acknowledgements(phase))
    }
    pub fn begin(&mut self, stamp: PublicationStamp) -> Result<()> {
        if !self.complete() {
            return Err(Error::State(
                "original caller left a phase traversal unfinished",
            ));
        }
        self.phase = Some(stamp.phase);
        self.epoch = stamp.epoch;
        self.next_ack = 0;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    FollowCameras,
    Land,
    Publish,
    PrepareViews,
    TerrainCaches,
    Input,
    Players,
    Shadows,
    Population,
    Elevators,
    Items,
    TomatoRain,
    Movement,
    ViewMembership,
    AnimationMotion,
    Speech,
}
const PRESENTATION: &[Operation] = &[
    Operation::FollowCameras,
    Operation::Land,
    Operation::Publish,
];
const PLAYERS: &[Operation] = &[
    Operation::PrepareViews,
    Operation::TerrainCaches,
    Operation::Input,
    Operation::Players,
    Operation::Shadows,
    Operation::Movement,
    Operation::ViewMembership,
    Operation::AnimationMotion,
    Operation::Speech,
];
const POPULATION: &[Operation] = &[
    Operation::PrepareViews,
    Operation::Population,
    Operation::Elevators,
    Operation::Items,
    Operation::TomatoRain,
    Operation::Movement,
    Operation::ViewMembership,
    Operation::AnimationMotion,
    Operation::Speech,
];

pub const fn operations(phase: Phase) -> &'static [Operation] {
    match phase {
        Phase::Players => PLAYERS,
        Phase::Population => POPULATION,
        Phase::PresentationA | Phase::PresentationB => PRESENTATION,
    }
}

/// Each operation must terminate after bounded deterministic work. Waiting and
/// original audio/device RTS calls belong to the enhanced resumable scheduler,
/// never a synchronous drain inside an operation. supports is a pure preflight.
pub trait PhaseServices {
    fn supports(&self, operation: Operation) -> bool;
    fn execute(
        &mut self,
        operation: Operation,
        session: &mut Session,
        stamp: PublicationStamp,
    ) -> Result<()>;
}

/// Reentry cannot replay service effects. A service failure latches the session,
/// matching the enhanced driver's no-fallback/no-retry execution contract.
pub fn run_phase(
    session: &mut Session,
    services: &mut impl PhaseServices,
    phase: Phase,
    input: InputFrame,
) -> Result<Admission> {
    if session.failed() {
        return Err(Error::State("mode-owned execution is latched failed"));
    }
    if operations(phase)
        .iter()
        .any(|&operation| !services.supports(operation))
    {
        return Err(Error::Ownership(
            "phase has an unimplemented service operation",
        ));
    }
    session.validate_inputs(&input)?;
    let admission = session.clock.admit(phase, input)?;
    let Admission::Step(stamp) = admission else {
        return Ok(admission);
    };
    for &operation in operations(phase) {
        if let Err(error) = services.execute(operation, session, stamp) {
            session.fail();
            return Err(error);
        }
    }
    if phase.publishes()
        && let Err(error) = session.publish(stamp)
    {
        session.fail();
        return Err(error);
    }
    Ok(admission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_relative_service_order_is_explicit() {
        assert_eq!(
            operations(Phase::PresentationA),
            operations(Phase::PresentationB)
        );
        assert_eq!(
            operations(Phase::Players),
            &[
                Operation::PrepareViews,
                Operation::TerrainCaches,
                Operation::Input,
                Operation::Players,
                Operation::Shadows,
                Operation::Movement,
                Operation::ViewMembership,
                Operation::AnimationMotion,
                Operation::Speech
            ]
        );
        assert_eq!(
            operations(Phase::Population),
            &[
                Operation::PrepareViews,
                Operation::Population,
                Operation::Elevators,
                Operation::Items,
                Operation::TomatoRain,
                Operation::Movement,
                Operation::ViewMembership,
                Operation::AnimationMotion,
                Operation::Speech
            ]
        );
        assert_eq!(
            operations(Phase::PresentationA),
            &[
                Operation::FollowCameras,
                Operation::Land,
                Operation::Publish
            ]
        );
    }
}
