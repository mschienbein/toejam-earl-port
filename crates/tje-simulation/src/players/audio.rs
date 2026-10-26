//! Logical requests resolved against the authenticated ROM by the resumable
//! device bridge. A request is acknowledged only after its real call completes.
use super::{PlayerError, PlayerResult};
use bincode::{Decode, Encode};
use tje_four_player::{Archetype, SeatId};
use tje_game::{FunctionId, ids};

pub const AUDIO_CAPACITY: usize = 128;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum AudioCue {
    Psg {
        command: u8,
        priority: u8,
    },
    Template {
        effect: u8,
    },
    Pcm {
        sound: PcmSound,
        priority: u8,
        mode: u8,
    },
    BoomboxStart,
    ResumeMusic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PcmSound {
    ToeJamYouch,
    EarlYouch,
    Squish,
    Aloha,
    Boogie,
    Burp,
    ChowDown,
    ExtraLife,
    Falling,
    Money,
    Randomize,
    Running,
    Snoring,
    Splash,
    Springing,
    Telephone,
    WakeShout,
    WakeWhisper,
    Hallelujah,
    Wiseman,
    Opera,
    ChickenCannon,
    Meep,
}
impl PcmSound {
    pub const fn youch(archetype: Archetype) -> Self {
        match archetype {
            Archetype::ToeJam => Self::ToeJamYouch,
            Archetype::Earl => Self::EarlYouch,
        }
    }
    fn symbol(self) -> &'static str {
        match self {
            Self::ToeJamYouch => "Sfx_ToejamYouch",
            Self::EarlYouch => "Sfx_EarlYouch",
            Self::Squish => "Sfx_Squish",
            Self::Aloha => "Sfx_Aloha",
            Self::Boogie => "Sfx_BoogieBoogie",
            Self::Burp => "Sfx_Burp",
            Self::ChowDown => "Sfx_ChowDown",
            Self::ExtraLife => "Sfx_ExtraLifeGlitz",
            Self::Falling => "Sfx_Falling",
            Self::Money => "Sfx_Money",
            Self::Randomize => "Sfx_Randomize",
            Self::Running => "Sfx_Running",
            Self::Snoring => "Sfx_Snoring",
            Self::Splash => "Sfx_Splash",
            Self::Springing => "Sfx_Springing",
            Self::Telephone => "Sfx_Telephone",
            Self::WakeShout => "Sfx_WakeUpShout",
            Self::WakeWhisper => "Sfx_WakeUpWhisper",
            Self::Hallelujah => "Sfx_Hallelujah",
            Self::Wiseman => "Sfx_WisemanThink",
            Self::Opera => "Sfx_OperaSing",
            Self::ChickenCannon => "Sfx_ChickenCannonFire",
            Self::Meep => "Sfx_Meep",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCommand {
    Original {
        function: FunctionId,
        arguments: [u32; 3],
        count: u8,
    },
    /// Publish to the retained audio owner's real requested-music field; the
    /// genuine IRQ/sequencer performs the change. This is not a synthetic RTS.
    RequestMusic { selection: u8 },
}
impl AudioCue {
    pub fn validate(self) -> PlayerResult<()> {
        let valid = match self {
            Self::Psg { command, priority } => command <= 26 && priority <= 127,
            Self::Template { effect } => matches!(effect, 4 | 11 | 13 | 14 | 15),
            Self::Pcm { priority, mode, .. } => priority <= 127 && mode <= 1,
            Self::BoomboxStart | Self::ResumeMusic => true,
        };
        if valid {
            Ok(())
        } else {
            Err(PlayerError::Invalid("audio cue arguments"))
        }
    }
    pub fn resolve(
        self,
        revision: tje_mem::Revision,
        rom_len: usize,
        normal_music: u8,
    ) -> PlayerResult<AudioCommand> {
        self.validate()?;
        Ok(match self {
            Self::Psg { command, priority } => AudioCommand::Original {
                function: ids::PLAY_SOUND,
                arguments: [u32::from(command), u32::from(priority), 0],
                count: 2,
            },
            Self::Template { effect } => AudioCommand::Original {
                function: ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
                arguments: [u32::from(effect), 0, 0],
                count: 1,
            },
            Self::Pcm {
                sound,
                priority,
                mode,
            } => {
                let resource = tje_mem::rom_symbol(revision, sound.symbol()).ok_or(
                    PlayerError::Rejected("audio PCM resource is not catalogued"),
                )?;
                if resource.address as usize >= rom_len {
                    return Err(PlayerError::Invalid("audio PCM resource extent"));
                }
                AudioCommand::Original {
                    function: ids::PLAY_SOUND_EFFECT,
                    arguments: [resource.address, u32::from(priority), u32::from(mode)],
                    count: 3,
                }
            }
            Self::BoomboxStart => AudioCommand::RequestMusic { selection: 14 },
            Self::ResumeMusic => {
                if normal_music > 14 {
                    return Err(PlayerError::Invalid("normal music selection"));
                }
                AudioCommand::RequestMusic {
                    selection: normal_music,
                }
            }
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct AudioIntent {
    pub sequence: u64,
    pub seat: SeatId,
    pub cue: AudioCue,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct AudioQueue {
    slots: [Option<AudioIntent>; AUDIO_CAPACITY],
    head: u16,
    length: u16,
    next_sequence: u64,
}
impl Default for AudioQueue {
    fn default() -> Self {
        Self {
            slots: [None; AUDIO_CAPACITY],
            head: 0,
            length: 0,
            next_sequence: 1,
        }
    }
}
impl AudioQueue {
    pub fn len(&self) -> usize {
        usize::from(self.length)
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn reserve(&self, count: usize) -> PlayerResult<()> {
        if self
            .len()
            .checked_add(count)
            .is_none_or(|total| total > AUDIO_CAPACITY)
            || self.next_sequence.checked_add(count as u64).is_none()
        {
            return Err(PlayerError::Capacity("pending audio intents"));
        }
        Ok(())
    }
    pub fn enqueue(&mut self, seat: SeatId, cue: AudioCue) -> PlayerResult<u64> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("audio owner"))?;
        cue.validate()?;
        self.reserve(1)?;
        let sequence = self.next_sequence;
        let index = (usize::from(self.head) + self.len()) % AUDIO_CAPACITY;
        self.slots[index] = Some(AudioIntent {
            sequence,
            seat,
            cue,
        });
        self.length += 1;
        self.next_sequence += 1;
        Ok(sequence)
    }
    pub fn front(&self) -> Option<&AudioIntent> {
        self.slots[usize::from(self.head)].as_ref()
    }
    pub fn acknowledge(&mut self, sequence: u64) -> PlayerResult<()> {
        if self
            .front()
            .is_none_or(|intent| intent.sequence != sequence)
        {
            return Err(PlayerError::Invalid(
                "audio acknowledgement is stale or out of order",
            ));
        }
        self.slots[usize::from(self.head)] = None;
        self.head = ((usize::from(self.head) + 1) % AUDIO_CAPACITY) as u16;
        self.length -= 1;
        Ok(())
    }
    pub fn validate(&self) -> PlayerResult<()> {
        if usize::from(self.head) >= AUDIO_CAPACITY
            || self.len() > AUDIO_CAPACITY
            || self.next_sequence == 0
        {
            return Err(PlayerError::Invalid("audio queue extent"));
        }
        let mut previous = 0;
        for offset in 0..AUDIO_CAPACITY {
            let entry = self.slots[(usize::from(self.head) + offset) % AUDIO_CAPACITY];
            if offset < self.len() {
                let intent = entry.ok_or(PlayerError::Invalid("audio queue hole"))?;
                intent.cue.validate()?;
                intent
                    .seat
                    .validate()
                    .map_err(|_| PlayerError::Invalid("audio queue seat"))?;
                if intent.sequence <= previous || intent.sequence >= self.next_sequence {
                    return Err(PlayerError::Invalid("audio queue sequence"));
                }
                previous = intent.sequence;
            } else if entry.is_some() {
                return Err(PlayerError::Invalid("audio outside queue"));
            }
        }
        Ok(())
    }
}
