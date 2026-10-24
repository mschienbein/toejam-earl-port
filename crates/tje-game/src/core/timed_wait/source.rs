//! Portable ordinary-return source for the exact eleven-role wait body.
use super::{WaitBranch, WaitRegisters, WaitRole, WaitSourceError, WaitStep};
use crate::{Error, Result, ids, timing::*};
use tje_mem::Revision;

const fn location(role: WaitRole) -> Location {
    Location {
        function: ids::WAIT_FOR_NEXT_V_BLANK,
        role: role as u16,
    }
}
const fn edge(from: WaitRole, to: WaitRole) -> Edge {
    Edge {
        from: location(from),
        to: location(to),
    }
}
static PROGRAM: Program = Program {
    name: "WaitForNextVBlank",
    source: "startup_and_main_loop.c:142; matching MCC68K source roles",
    entry: location(WaitRole::LoadCount),
    functions: &[FunctionRegion {
        function: ids::WAIT_FOR_NEXT_V_BLANK,
        roles: 11,
    }],
    edges: &[
        edge(WaitRole::LoadCount, WaitRole::TestDisplay),
        edge(WaitRole::TestDisplay, WaitRole::DisplayBranch),
        edge(WaitRole::DisplayBranch, WaitRole::Return),
        edge(WaitRole::DisplayBranch, WaitRole::ToCountTest),
        edge(WaitRole::ToCountTest, WaitRole::TestCount),
        edge(WaitRole::CaptureCounter, WaitRole::CompareCounter),
        edge(WaitRole::CompareCounter, WaitRole::PollBranch),
        edge(WaitRole::PollBranch, WaitRole::CompareCounter),
        edge(WaitRole::PollBranch, WaitRole::DecrementCount),
        edge(WaitRole::DecrementCount, WaitRole::TestCount),
        edge(WaitRole::TestCount, WaitRole::CountBranch),
        edge(WaitRole::CountBranch, WaitRole::CaptureCounter),
        edge(WaitRole::CountBranch, WaitRole::Return),
    ],
};

pub struct WaitSource {
    revision: Revision,
    step: Option<WaitStep>,
    registers: Option<Registers>,
    pending: Option<Effect>,
    failed: bool,
}
impl WaitSource {
    pub const fn definition() -> &'static Program {
        &PROGRAM
    }
    pub fn new(revision: Revision) -> Self {
        Self {
            revision,
            step: None,
            registers: None,
            pending: None,
            failed: false,
        }
    }
    fn error(&mut self, message: &str) -> Error {
        self.failed = true;
        Error::Platform(message.into())
    }
}
impl TimingSource for WaitSource {
    fn program(&self) -> &'static Program {
        &PROGRAM
    }

    fn begin_instruction(&mut self, site: InstructionSite, registers: Registers) -> Result<()> {
        if self.failed
            || self.step.is_some()
            || self.pending.is_some()
            || site.location.function != ids::WAIT_FOR_NEXT_V_BLANK
        {
            return Err(self.error("wait source has a stale instruction or wrong function"));
        }
        let Some(&role) = WaitRole::ALL.get(usize::from(site.location.role)) else {
            return Err(self.error("wait source role is not declared"));
        };
        self.step = Some(WaitStep::begin(
            self.revision,
            role,
            WaitRegisters {
                d0: registers.data[0],
                d1: registers.data[1],
                sr: u32::from(registers.status),
                sp: registers.stack_pointer(),
            },
        ));
        self.registers = Some(registers);
        Ok(())
    }
    fn next_effect(&mut self) -> Result<Option<Effect>> {
        if self.failed || self.pending.is_some() {
            return Err(self.error("wait source effect requires its actual acknowledgement"));
        }
        let Some(step) = &self.step else {
            return Err(self.error("wait source instruction has not begun"));
        };
        let effect = match step.expected_read() {
            Ok(read) => Some(Effect {
                address: read.address,
                width: if read.word { Width::Word } else { Width::Byte },
                kind: EffectKind::Read(ReadPurpose::Semantic),
            }),
            Err(WaitSourceError::ExtraRead) => None,
            Err(error) => return Err(self.error(&error.to_string())),
        };
        self.pending = effect;
        Ok(effect)
    }
    fn acknowledge(&mut self, actual: Observation) -> Result<()> {
        let mask = if actual.effect.width == Width::Word {
            u16::MAX
        } else {
            255
        };
        if self.failed || self.pending != Some(actual.effect) || actual.value > mask {
            return Err(self.error("wait source acknowledgement is missing, repeated or changed"));
        }
        let read = super::SourceRead {
            address: actual.effect.address,
            word: actual.effect.width == Width::Word,
        };
        let result = self
            .step
            .as_mut()
            .expect("pending instruction")
            .acknowledge_read(read, u32::from(actual.value));
        if let Err(error) = result {
            return Err(self.error(&error.to_string()));
        }
        self.pending = None;
        Ok(())
    }
    fn finish_instruction(&mut self) -> Result<Commit> {
        if self.failed || self.pending.is_some() {
            return Err(self.error("wait source cannot finish with an unacknowledged effect"));
        }
        let Some(step) = self.step.take() else {
            return Err(self.error("wait source instruction already finished"));
        };
        let effect = step
            .finish()
            .map_err(|error| self.error(&error.to_string()))?;
        let mut registers = self.registers.take().expect("begun instruction registers");
        registers.data[0] = effect.registers.d0;
        registers.data[1] = effect.registers.d1;
        registers.status = effect.registers.sr as u16;
        registers.set_stack_pointer(effect.registers.sp);
        let path = match effect.next {
            WaitBranch::Role(role) => Path::Next(location(role)),
            WaitBranch::Return(address) => Path::Return {
                address,
                value: None,
            },
        };
        Ok(Commit { registers, path })
    }
}
