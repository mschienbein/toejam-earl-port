//! Admission-only traversal of the actual sequencer bytecode grammar. Only
//! control-flow-reachable opcode-seven operands are callback addresses.
use super::{PlayerError, PlayerResult};
use tje_game::{FunctionId, ids};
use tje_mem::{Revision, RomDataUnit, rom_data_units};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAudioCatalog {
    callbacks: Vec<FunctionId>,
    pub roots: u16,
    pub commands: u32,
}
impl CheckedAudioCatalog {
    pub fn callbacks(&self) -> &[FunctionId] {
        &self.callbacks
    }
    pub fn from_rom(rom: &[u8], revision: Revision) -> PlayerResult<Self> {
        let songs = unit(revision, "SongChannelTemplates")?;
        let effects = unit(revision, "SfxChannelTemplates")?;
        if songs.size % 228 != 0
            || songs.size / 228 > 32
            || effects.size % 38 != 0
            || effects.size / 38 > 128
        {
            return Err(PlayerError::Invalid("audio template extent"));
        }
        let music: Vec<_> = rom_data_units(revision)
            .iter()
            .filter(|unit| unit.kind == "music")
            .copied()
            .collect();
        if music.is_empty()
            || music.len() > 8
            || music.iter().map(|unit| u64::from(unit.size)).sum::<u64>() > 128 * 1024
        {
            return Err(PlayerError::Invalid("music stream extent"));
        }
        let mut roots = Vec::with_capacity(256);
        let mut callbacks = Vec::new();
        for template in [songs, effects] {
            let bytes = extent(rom, template.address, template.size as usize)?;
            for record in bytes.as_chunks::<38>().0 {
                let stream = u32::from_be_bytes(record[4..8].try_into().expect("channel stream"));
                let callback =
                    u32::from_be_bytes(record[8..12].try_into().expect("channel callback"));
                callback_id(revision, callback, &mut callbacks)?;
                if stream != 0 && !roots.contains(&stream) {
                    roots.push(stream);
                }
            }
        }
        // Auxiliary effects select bounded records through a pointer/count
        // table, not arbitrary caller-supplied ROM offsets.
        let pointers = unit(revision, "SfxChannelTemplatePointers")?;
        let counts = unit(revision, "SfxChannelTemplateCounts")?;
        if pointers.size / 4 != counts.size || pointers.size % 4 != 0 || counts.size > 128 {
            return Err(PlayerError::Invalid("effect template selector extent"));
        }
        let pointer_bytes = extent(rom, pointers.address, pointers.size as usize)?;
        let count_bytes = extent(rom, counts.address, counts.size as usize)?;
        for (&count, pointer) in count_bytes
            .iter()
            .zip(pointer_bytes.as_chunks::<4>().0)
            .skip(1)
        {
            let address = u32::from_be_bytes(*pointer);
            let size = usize::from(count) * 38;
            if count == 0
                || count > 3
                || address < effects.address
                || u64::from(address) + size as u64
                    > u64::from(effects.address) + u64::from(effects.size)
                || (address - effects.address) % 38 != 0
            {
                return Err(PlayerError::Invalid("effect template pointer/count"));
            }
        }
        let mut marks: Vec<Vec<u8>> = music
            .iter()
            .map(|unit| vec![0; unit.size as usize])
            .collect();
        let mut queue = roots.clone();
        let mut cursor = 0;
        let mut commands = 0_u32;
        while cursor < queue.len() {
            let address = queue[cursor];
            cursor += 1;
            let index = music
                .iter()
                .position(|unit| {
                    address >= unit.address
                        && u64::from(address) < u64::from(unit.address) + u64::from(unit.size)
                })
                .ok_or(PlayerError::Invalid(
                    "music jump outside declared stream units",
                ))?;
            let stream = music[index];
            let offset = (address - stream.address) as usize;
            match marks[index][offset] {
                1 => continue,
                2 => return Err(PlayerError::Invalid("music branch targets an operand")),
                _ => {}
            }
            let opcode = extent(rom, address, 1)?[0];
            let after = address
                .checked_add(1)
                .ok_or(PlayerError::Invalid("music PC overflow"))?;
            let length = match opcode {
                0 | 2 | 9..=12 | 0x80..=0xff => 1,
                1 | 3 | 5 | 6 | 8 => 3,
                4 => 2,
                7 => 1 + usize::from(after & 1 != 0) + 4,
                _ => return Err(PlayerError::Invalid("unsupported music opcode")),
            };
            if offset + length > marks[index].len() {
                return Err(PlayerError::Invalid("truncated music command"));
            }
            for operand in &mut marks[index][offset + 1..offset + length] {
                if *operand == 1 {
                    return Err(PlayerError::Invalid("music operand overlaps a command"));
                }
                *operand = 2;
            }
            marks[index][offset] = 1;
            let bytes = extent(rom, address, length)?;
            commands += 1;
            let next = address
                .checked_add(length as u32)
                .ok_or(PlayerError::Invalid("music PC overflow"))?;
            let mut push = |target: u32| -> PlayerResult<()> {
                if queue.len() >= 256 * 1024 {
                    return Err(PlayerError::Capacity("music callback control-flow audit"));
                }
                queue.push(target);
                Ok(())
            };
            match opcode {
                0 => {}
                6 => {
                    let relative = i32::from(i16::from_be_bytes([bytes[1], bytes[2]]));
                    let target = i64::from(next) + 3 + i64::from(relative);
                    if !(0..=i64::from(u32::MAX)).contains(&target) {
                        return Err(PlayerError::Invalid("music loop target overflow"));
                    }
                    push(target as u32)?;
                    push(next)?;
                }
                8 => {
                    let relative = i32::from(i16::from_be_bytes([bytes[1], bytes[2]]));
                    let target = i64::from(next) + i64::from(relative);
                    if !(0..=i64::from(u32::MAX)).contains(&target) {
                        return Err(PlayerError::Invalid("music jump overflow"));
                    }
                    push(target as u32)?;
                }
                7 => {
                    let begin = 1 + usize::from(after & 1 != 0);
                    callback_id(
                        revision,
                        u32::from_be_bytes(
                            bytes[begin..begin + 4]
                                .try_into()
                                .expect("callback operand"),
                        ),
                        &mut callbacks,
                    )?;
                    push(next)?;
                }
                _ => push(next)?,
            }
        }
        callbacks.sort_by_key(|id| id.metadata().inventory_id);
        Ok(Self {
            callbacks,
            roots: roots.len() as u16,
            commands,
        })
    }
}
fn callback_id(
    revision: Revision,
    address: u32,
    callbacks: &mut Vec<FunctionId>,
) -> PlayerResult<()> {
    if address == 0 {
        return Ok(());
    }
    let id = tje_game::function_at(revision, address)
        .ok_or(PlayerError::Rejected("unknown or interior music callback"))?;
    // This is the recovered no-argument, channel-register callback contract.
    // Other audio roots are not interchangeable merely because they return RTS.
    if id != ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK {
        return Err(PlayerError::Rejected("unapproved music callback ABI"));
    }
    if !callbacks.contains(&id) {
        callbacks.push(id);
    }
    Ok(())
}
fn unit(revision: Revision, name: &str) -> PlayerResult<RomDataUnit> {
    rom_data_units(revision)
        .iter()
        .find(|unit| unit.name == name)
        .copied()
        .ok_or(PlayerError::Invalid("missing audio data unit"))
}
fn extent(rom: &[u8], address: u32, length: usize) -> PlayerResult<&[u8]> {
    let start = address as usize;
    let end = start
        .checked_add(length)
        .ok_or(PlayerError::Invalid("audio extent overflow"))?;
    rom.get(start..end)
        .ok_or(PlayerError::Invalid("audio extent outside user ROM"))
}
