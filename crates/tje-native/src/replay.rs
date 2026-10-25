//! Exact controller-session playback shared by the windowed game and its proof.
use crate::{backend::GameBackend, input::Controllers};
use anyhow::{Context, Result, ensure};
use std::path::Path;
use tje_rom::Revision;
use tje_session::{CONNECTED, Run, Session};

pub struct Replay {
    session: Session,
    frame: u64,
    run: usize,
    offset: u64,
    write: usize,
}

pub struct PlayedFrame<'a> {
    pub audio: &'a [[f64; 2]],
    pub controllers: Controllers,
}

impl Replay {
    pub fn load(path: &Path) -> Result<Self> {
        Self::new(Session::from_json(
            &std::fs::read_to_string(path)
                .with_context(|| format!("read input session {}", path.display()))?,
        )?)
    }
    pub fn new(session: Session) -> Result<Self> {
        session.validate()?;
        ensure!(session.frames > 0, "cannot replay an empty session");
        Ok(Self {
            session,
            frame: 0,
            run: 0,
            offset: 0,
            write: 0,
        })
    }
    pub fn revision(&self) -> Revision {
        self.session.revision.into()
    }
    pub fn frames(&self) -> u64 {
        self.session.frames
    }
    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn finished(&self) -> bool {
        self.frame == self.session.frames
    }
    pub fn session(&self) -> &Session {
        &self.session
    }
    pub fn restart(&mut self) {
        self.frame = 0;
        self.run = 0;
        self.offset = 0;
        self.write = 0;
    }
    pub fn pads(&self) -> Option<[u16; 2]> {
        let &Run(_, p1, p2) = self.session.runs.get(self.run)?;
        Some([p1, p2])
    }
    pub fn controllers(&self) -> Option<Controllers> {
        self.pads().map(controllers)
    }
    /// Pausing never calls this method. A successful call consumes exactly the
    /// inputs for one console frame, including explicit recipe writes, if any.
    pub fn advance<'a>(
        &mut self,
        backend: &'a mut dyn GameBackend,
    ) -> Result<Option<PlayedFrame<'a>>> {
        let Some(controllers) = self.controllers() else {
            return Ok(None);
        };
        while let Some(write) = self.session.writes().get(self.write) {
            if write.before_frame != self.frame {
                break;
            }
            backend.write_ram(write)?;
            self.write += 1;
        }
        let audio = backend.advance(controllers)?;
        self.frame += 1;
        self.offset += 1;
        if self.offset == self.session.runs[self.run].0 {
            self.run += 1;
            self.offset = 0;
        }
        Ok(Some(PlayedFrame { audio, controllers }))
    }
}

pub fn controllers(pads: [u16; 2]) -> Controllers {
    Controllers {
        connected: pads.map(|pad| pad & CONNECTED != 0),
        buttons: pads.map(|pad| std::array::from_fn(|bit| pad & (1 << bit) != 0)),
        jam: [false; 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_boundaries_connections_and_restart_are_exact() {
        let mut session = Session::new(Revision::Rev00, "authored cursor test");
        session.record([CONNECTED, 0]).unwrap();
        session.record([CONNECTED, 0]).unwrap();
        session.record([CONNECTED | 8, CONNECTED | 128]).unwrap();
        let mut replay = Replay::new(session).unwrap();
        assert_eq!(replay.revision(), Revision::Rev00);
        assert_eq!(replay.frames(), 3);
        assert_eq!(replay.controllers().unwrap().connected, [true, false]);
        replay.frame = 2;
        replay.run = 1;
        assert!(replay.controllers().unwrap().buttons[0][3]);
        assert!(replay.controllers().unwrap().buttons[1][7]);
        replay.restart();
        assert_eq!(replay.frame(), 0);
        assert_eq!(replay.controllers().unwrap().connected, [true, false]);
    }
    #[test]
    fn empty_replay_is_rejected() {
        assert!(Replay::new(Session::new(Revision::Rev02, "empty")).is_err());
    }
}
