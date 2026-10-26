//! Dedicated predicates consume the actual original poll stream including its
//! terminal read. Authenticate test/mask/operand/branch, never truth coincidence.
use super::{
    Failure, Unsupported, UnsupportedKind,
    cursor::{Cursor, Token},
    record::{Direction, Instruction},
};
use m68000_emu::ReadPurpose;
use tje_game::execution::{RamWidth, WaitComparison, WaitCondition, WaitRequest};

#[derive(Clone, Copy)]
struct Predicate {
    address: u32,
    width: RamWidth,
    mask: u32,
    value: u32,
    comparison: WaitComparison,
}
fn unsupported(detail: impl Into<String>) -> Failure {
    Failure::Unsupported(Unsupported {
        kind: UnsupportedKind::Observation,
        detail: detail.into(),
    })
}
fn request(request: WaitRequest) -> Result<Predicate, Failure> {
    let predicate = match request.condition {
        WaitCondition::Ram {
            address,
            width,
            mask,
            value,
            comparison,
        } => {
            if !width.valid(address, mask, value) {
                return Err(Failure::Mismatch("invalid RAM wait predicate".into()));
            }
            Predicate {
                address,
                width,
                mask,
                value,
                comparison,
            }
        }
        WaitCondition::RamWordChanged { address, previous } => Predicate {
            address,
            width: RamWidth::Word,
            mask: 65535,
            value: u32::from(previous),
            comparison: WaitComparison::NotEqual,
        },
        WaitCondition::DeviceWord {
            address,
            mask,
            value,
        } => Predicate {
            address,
            width: RamWidth::Word,
            mask: u32::from(mask),
            value: u32::from(value),
            comparison: WaitComparison::Equal,
        },
        WaitCondition::DeviceByte {
            address,
            mask,
            value,
        } => Predicate {
            address,
            width: RamWidth::Byte,
            mask: u32::from(mask),
            value: u32::from(value),
            comparison: WaitComparison::Equal,
        },
        WaitCondition::Interrupt => {
            return Err(unsupported(
                "interrupt-only suspension requires a code-authenticated control profile; compound spins use exact reads plus poll_yield",
            ));
        }
        WaitCondition::VdpFillComplete => {
            return Err(unsupported(
                "non-observing fill join requires its authenticated activation/completion control profile",
            ));
        }
    };
    Ok(Predicate {
        address: predicate.address & 0xffffff,
        ..predicate
    })
}
fn word(reader: &dyn Fn(u32) -> Option<u16>, pc: u32) -> Result<u16, Failure> {
    reader(pc).ok_or_else(|| {
        unsupported(format!(
            "poll certificate lacks immutable instruction word at {pc:06x}"
        ))
    })
}
fn immediate(
    reader: &dyn Fn(u32) -> Option<u16>,
    pc: u32,
    width: RamWidth,
) -> Result<u32, Failure> {
    let high = u32::from(word(reader, pc)?);
    Ok(match width {
        RamWidth::Byte => high & 255,
        RamWidth::Word => high,
        RamWidth::Long => (high << 16) | u32::from(word(reader, pc + 2)?),
    })
}
fn size(bits: u16) -> Result<RamWidth, Failure> {
    match bits & 3 {
        0 => Ok(RamWidth::Byte),
        1 => Ok(RamWidth::Word),
        2 => Ok(RamWidth::Long),
        _ => Err(unsupported(
            "poll test has no ordinary byte/word/long operand",
        )),
    }
}
fn side_effect_free_ea(opcode: u16) -> bool {
    let mode = (opcode >> 3) & 7;
    mode >= 2 && mode != 3 && mode != 4 && !(mode == 7 && opcode & 7 > 3)
}
fn branch(
    reader: &dyn Fn(u32) -> Option<u16>,
    instruction: &Instruction,
) -> Result<(u32, u32, WaitComparison), Failure> {
    let opcode = instruction.actual.opcode;
    let comparison = match opcode >> 8 {
        0x66 => WaitComparison::Equal,
        0x67 => WaitComparison::NotEqual,
        _ => {
            return Err(unsupported(
                "dedicated poll has no immediate BEQ/BNE loop branch",
            ));
        }
    };
    let (displacement, length) = if opcode as u8 == 0 {
        (
            i32::from(word(reader, instruction.actual.pc + 2)? as i16),
            4,
        )
    } else {
        (i32::from(opcode as u8 as i8), 2)
    };
    Ok((
        instruction
            .actual
            .pc
            .wrapping_add(2)
            .wrapping_add_signed(displacement),
        instruction.actual.pc.wrapping_add(length),
        comparison,
    ))
}
fn pure_tail(cursor: &Cursor<'_>, instruction: &Instruction) -> bool {
    cursor.original.trace.issues[instruction.issues.clone()]
        .iter()
        .all(|access| access.direction == Direction::Read(ReadPurpose::Prefetch))
}
fn certificate<'a>(
    cursor: &Cursor<'a>,
    predicate: Predicate,
    reader: &dyn Fn(u32) -> Option<u16>,
) -> Result<(&'a Instruction, u32), Failure> {
    let path = cursor.instruction_span();
    let poll = path[0].ok_or_else(|| unsupported("dedicated wait is not at an actual own poll"))?;
    let opcode = poll.actual.opcode;
    if !side_effect_free_ea(opcode) {
        return Err(unsupported(
            "poll EA changes an address register or is not a memory operand",
        ));
    }
    let width;
    let mut mask;
    let mut expected;
    let mut tail = 1;
    if opcode & 0xff00 == 0x4a00 {
        width = size(opcode >> 6)?;
        mask = width.mask();
        expected = 0;
    } else if opcode & 0xff00 == 0x0c00 {
        width = size(opcode >> 6)?;
        mask = width.mask();
        expected = immediate(reader, poll.actual.pc + 2, width)?;
    } else if opcode & 0xf100 == 0xb000 && ((opcode >> 6) & 7) <= 2 {
        width = size(opcode >> 6)?;
        mask = width.mask();
        expected = poll.before.cpu.words[usize::from((opcode >> 9) & 7)] & width.mask();
    } else if opcode & 0xffc0 == 0x0800 {
        width = RamWidth::Byte;
        mask = 1 << (word(reader, poll.actual.pc + 2)? & 7);
        expected = 0;
    } else if opcode & 0xf1c0 == 0x0100 {
        width = RamWidth::Byte;
        mask = 1 << (poll.before.cpu.words[usize::from((opcode >> 9) & 7)] & 7);
        expected = 0;
    } else if matches!(opcode >> 12, 1..=3) && (opcode >> 6) & 7 == 0 {
        width = match opcode >> 12 {
            1 => RamWidth::Byte,
            2 => RamWidth::Long,
            _ => RamWidth::Word,
        };
        mask = width.mask();
        expected = 0;
        let register = opcode >> 9 & 7;
        if let Some(next) = path[tail]
            && next.actual.opcode & 0xff38 == 0x0200
            && next.actual.opcode & 7 == register
            && size(next.actual.opcode >> 6)? == width
        {
            if !pure_tail(cursor, next) {
                return Err(unsupported("masked poll tail has additional own accesses"));
            }
            mask = immediate(reader, next.actual.pc + 2, width)?;
            tail += 1;
        }
        if let Some(next) = path[tail]
            && next.actual.opcode & 0xff38 == 0x0c00
            && next.actual.opcode & 7 == register
            && size(next.actual.opcode >> 6)? == width
        {
            if !pure_tail(cursor, next) {
                return Err(unsupported("compare poll tail has additional own accesses"));
            }
            expected = immediate(reader, next.actual.pc + 2, width)?;
            tail += 1;
        } else if let Some(next) = path[tail]
            && next.actual.opcode & 0xff38 == 0x4a00
            && next.actual.opcode & 7 == register
            && size(next.actual.opcode >> 6)? == width
        {
            if !pure_tail(cursor, next) {
                return Err(unsupported("test poll tail has additional own accesses"));
            }
            tail += 1;
        }
    } else {
        return Err(unsupported(
            "poll shape is not a reviewed single-read predicate; use literal accesses plus poll_yield",
        ));
    }
    let branch_instruction = path
        .get(tail)
        .copied()
        .flatten()
        .ok_or_else(|| unsupported("poll certificate has no actual loop branch"))?;
    if !pure_tail(cursor, branch_instruction) {
        return Err(unsupported("poll branch contains additional own accesses"));
    }
    let (target, fallthrough, comparison) = branch(reader, branch_instruction)?;
    if target != poll.actual.pc {
        return Err(unsupported(
            "poll branch does not return to the same exact read/test",
        ));
    }
    if width != predicate.width
        || mask != predicate.mask
        || expected != predicate.value
        || comparison != predicate.comparison
    {
        return Err(Failure::Mismatch(format!(
            "wrong wait predicate: actual code width={width:?}, mask={mask:08x}, value={expected:08x}, relation={comparison:?}"
        )));
    }
    Ok((branch_instruction, fallthrough))
}

pub(super) fn run(
    inner: &mut super::context::Inner<'_>,
    request_: WaitRequest,
) -> Result<u32, Failure> {
    let predicate = request(request_)?;
    let reader = inner.reader;
    let mut first_tick = None;
    let mut poll_pc = None;
    loop {
        let token = inner.peek(false)?;
        let Token::Access { access, pc, .. } = token else {
            return Err(unsupported(
                "dedicated wait has no next actual semantic poll",
            ));
        };
        if access.address != predicate.address
            || access.word != !matches!(predicate.width, RamWidth::Byte)
            || access.direction != Direction::Read(ReadPurpose::Semantic)
        {
            return Err(Failure::Mismatch(
                "wait changed the exact poll address/width/purpose".into(),
            ));
        }
        if poll_pc.is_some_and(|previous| previous != pc) {
            return Err(Failure::Mismatch(
                "wait crossed into a different test/read site".into(),
            ));
        }
        poll_pc = Some(pc);
        let path = inner.cursor.instruction_span();
        let poll = path[0].ok_or_else(|| unsupported("poll has no own instruction witness"))?;
        let tick = poll.before.ticks;
        let start = *first_tick.get_or_insert(tick);
        if tick
            .checked_sub(start)
            .ok_or_else(|| unsupported("poll scheduler lineage moved backwards"))?
            > request_.max_ticks
        {
            return Err(Failure::Unsupported(Unsupported {
                kind: UnsupportedKind::OperationalCeiling,
                detail: "dedicated wait exceeded its unchanged operational scheduler-step ceiling"
                    .into(),
            }));
        }
        let (branch_instruction, fallthrough) = certificate(&inner.cursor, predicate, reader)?;
        let after_pc = branch_instruction.after_pc;
        let target_pc = poll.actual.pc;
        let first_mask = if predicate.width == RamWidth::Long {
            predicate.mask >> 16
        } else {
            predicate.mask
        };
        let high = u32::from(inner.poll_read(
            predicate.address,
            !matches!(predicate.width, RamWidth::Byte),
            first_mask as u16,
        )?);
        let sample = if predicate.width == RamWidth::Long {
            (high << 16)
                | u32::from(inner.poll_read(
                    predicate.address.wrapping_add(2),
                    true,
                    predicate.mask as u16,
                )?)
        } else {
            high
        };
        let satisfied = predicate
            .comparison
            .satisfied(sample, predicate.mask, predicate.value);
        if after_pc != if satisfied { fallthrough } else { target_pc } {
            return Err(Failure::Mismatch(
                "actual poll branch disagrees with the authenticated requested predicate".into(),
            ));
        }
        if satisfied {
            return Ok(sample);
        }
        #[cfg(test)]
        {
            inner.waited = true;
        }
    }
}
