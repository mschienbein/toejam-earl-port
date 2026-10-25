use crate::input::Controllers;
use anyhow::{Context, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tje_rom::Revision;
use tje_session::{Session, encode_pad};

#[derive(Default)]
pub struct Recorder {
    session: Option<Session>,
}
impl Recorder {
    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    /// The caller hard-resets the backend before the next frame: all saved input
    /// is consequently replayable from power-on, never from an opaque snapshot.
    pub fn start(&mut self, revision: Revision) {
        self.session = Some(Session::new(
            revision,
            "Native F9 controller recording from power-on",
        ));
    }

    pub fn frame(&mut self, controllers: Controllers) -> Result<()> {
        if let Some(session) = &mut self.session {
            session.record(std::array::from_fn(|index| {
                encode_pad(controllers.connected[index], controllers.buttons[index])
            }))?;
        }
        Ok(())
    }

    pub fn stop(&mut self, data_directory: &Path) -> Result<Option<PathBuf>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        if session.frames == 0 {
            self.session = None;
            return Ok(None);
        }
        let directory = data_directory.join("sessions");
        fs::create_dir_all(&directory).context("create recording directory")?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("session-{nonce}-{}.tje.json", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("create session {}", path.display()))?;
        file.write_all(session.to_json()?.as_bytes())
            .context("write input-only recording")?;
        file.sync_all().context("flush input-only recording")?;
        self.session = None;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_both_pads_and_no_machine_state() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("tje-recorder-{nonce}"));
        let mut recorder = Recorder::default();
        recorder.start(Revision::Rev00);
        let controllers = Controllers {
            connected: [true, false],
            buttons: [[true; 8]; 2],
            jam: [false; 2],
        };
        recorder.frame(controllers).unwrap();
        recorder.frame(controllers).unwrap();
        let path = recorder.stop(&directory).unwrap().unwrap();
        let session = Session::from_json(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(session.frames, 2);
        assert_eq!(session.runs, [tje_session::Run(2, 0x1FF, 0)]);
        assert!(!recorder.active());
        assert!(recorder.stop(&directory).unwrap().is_none());
        fs::remove_dir_all(directory).unwrap();
    }
}
