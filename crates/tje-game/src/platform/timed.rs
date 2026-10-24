//! Source-owned clocked platform leaves; no opcode, clock or emulator dependency.
//!
//! These ctx-only candidates are deliberately absent from production registries.
//! Their legacy entries retain their reviewed Atomic/Functional contracts. A
//! private ClockedStrict proof must exercise both sides before any cutover.
use crate::{
    Context, Error, FunctionId, Result, ids,
    timing::{
        Commit, Edge, Effect, EffectKind, FunctionRegion, InstructionSite, Location, Observation,
        Path, Program, ReadPurpose, Registers, TimingSource, Width,
    },
};
use tje_mem::{Revision, ram};

const VDP_DATA: u32 = 0xc00000;
const VDP_CONTROL: u32 = 0xc00004;
const Z80_BUS_REQUEST: u32 = 0xa11100;
const Z80_DAC_DELAY: u32 = 0xa0004a;
const Z80_PSG_ACTIVE: u32 = 0xa0004b;

const fn location(function: FunctionId, role: u16) -> Location {
    Location { function, role }
}

// Authored source control edges, not an opcode listing or a cycle table.
const fn forward_edges<const N: usize>(function: FunctionId) -> [Edge; N] {
    let mut edges = [Edge {
        from: location(function, 0),
        to: location(function, 1),
    }; N];
    let mut index = 0;
    while index < N {
        edges[index] = Edge {
            from: location(function, index as u16),
            to: location(function, index as u16 + 1),
        };
        index += 1;
    }
    edges
}

const fn controller_edges(function: FunctionId) -> [Edge; 21] {
    let mut edges = forward_edges(function);
    edges[19] = Edge {
        from: location(function, 4),
        to: location(function, 4),
    };
    edges[20] = Edge {
        from: location(function, 11),
        to: location(function, 11),
    };
    edges
}

const fn word_stream_edges() -> [Edge; 6] {
    let mut edges = forward_edges(ids::MOVE_DATA_TO_VRAM);
    edges[5] = Edge {
        from: location(ids::MOVE_DATA_TO_VRAM, 4),
        to: location(ids::MOVE_DATA_TO_VRAM, 3),
    };
    edges
}

const fn palette_edges() -> [Edge; 11] {
    let mut edges = forward_edges(ids::COPY_PALETTE_TO_CRAM_SLOT);
    edges[10] = Edge {
        from: location(ids::COPY_PALETTE_TO_CRAM_SLOT, 9),
        to: location(ids::COPY_PALETTE_TO_CRAM_SLOT, 8),
    };
    edges
}

static CONTROLLER1: Program = Program {
    name: "GetCtrl1Input source roles",
    source: "G/src/asm/controllers.s:12-34 (GetCtrl1Input)",
    entry: location(ids::GET_CTRL1_INPUT, 0),
    functions: &[FunctionRegion {
        function: ids::GET_CTRL1_INPUT,
        roles: 20,
    }],
    edges: &controller_edges(ids::GET_CTRL1_INPUT),
};
static CONTROLLER2: Program = Program {
    name: "GetCtrl2Input source roles",
    source: "G/src/asm/controllers.s:40-62 (GetCtrl2Input)",
    entry: location(ids::GET_CTRL2_INPUT, 0),
    functions: &[FunctionRegion {
        function: ids::GET_CTRL2_INPUT,
        roles: 20,
    }],
    edges: &controller_edges(ids::GET_CTRL2_INPUT),
};
static WORD_STREAM: Program = Program {
    name: "MoveDataToVRAM source roles",
    source: "G/src/asm/vdp.s:103-110 (MoveDataToVRAM)",
    entry: location(ids::MOVE_DATA_TO_VRAM, 0),
    functions: &[FunctionRegion {
        function: ids::MOVE_DATA_TO_VRAM,
        roles: 6,
    }],
    edges: &word_stream_edges(),
};
static PALETTE: Program = Program {
    name: "CopyPaletteToCRAMSlot source roles",
    source: "G/src/asm/vdp.s:153-165 (CopyPaletteToCRAMSlot)",
    entry: location(ids::COPY_PALETTE_TO_CRAM_SLOT, 0),
    functions: &[FunctionRegion {
        function: ids::COPY_PALETTE_TO_CRAM_SLOT,
        roles: 11,
    }],
    edges: &palette_edges(),
};
static DMA: Program = Program {
    name: "DmaCopyWordsToVram source roles",
    source: "G/src/asm/dma.s:15-75 (DmaCopyWordsToVram)",
    entry: location(ids::DMA_COPY_WORDS_TO_VRAM, 0),
    functions: &[FunctionRegion {
        function: ids::DMA_COPY_WORDS_TO_VRAM,
        roles: 54,
    }],
    edges: &forward_edges::<53>(ids::DMA_COPY_WORDS_TO_VRAM),
};

/// The five G assembly leaves with complete portable semantic instruction roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaf {
    Controller1,
    Controller2,
    WordStream,
    Palette,
    Dma,
}

impl Leaf {
    fn program(self) -> &'static Program {
        match self {
            Self::Controller1 => &CONTROLLER1,
            Self::Controller2 => &CONTROLLER2,
            Self::WordStream => &WORD_STREAM,
            Self::Palette => &PALETTE,
            Self::Dma => &DMA,
        }
    }

    fn is_controller(self) -> bool {
        matches!(self, Self::Controller1 | Self::Controller2)
    }
}

struct Instruction {
    role: u16,
    registers: Registers,
    acknowledged: usize,
    observations: [u16; 7],
    pending: Option<Effect>,
}

/// G/src/asm/controllers.s:12,40; vdp.s:103,153; dma.s:15.
/// Every semantic/discarded bus effect is acknowledged exactly once; no eager
/// argument snapshot, shadow RAM, original-callee execution or host clock exists.
pub struct Source {
    revision: Revision,
    leaf: Leaf,
    next: Location,
    instruction: Option<Instruction>,
    finished: bool,
    poisoned: bool,
}

impl Source {
    pub fn new(revision: Revision, leaf: Leaf) -> Self {
        Self {
            revision,
            leaf,
            next: leaf.program().entry,
            instruction: None,
            finished: false,
            poisoned: false,
        }
    }

    fn reject(&mut self, reason: &'static str) -> Error {
        self.poisoned = true;
        Error::Platform(reason.into())
    }

    fn effect(&self, instruction: &Instruction) -> Option<Effect> {
        let index = instruction.acknowledged;
        let registers = &instruction.registers;
        let sp = registers.stack_pointer();
        let read = |address, width, purpose| Effect {
            address,
            width,
            kind: EffectKind::Read(purpose),
        };
        let write = |address, width, value| Effect {
            address,
            width,
            kind: EffectKind::Write(value),
        };
        let read_word = |address| read(address, Width::Word, ReadPurpose::Semantic);
        let read_byte = |address| read(address, Width::Byte, ReadPurpose::Semantic);
        let write_word = |address, value| write(address, Width::Word, value);
        let write_byte = |address, value| write(address, Width::Byte, value);
        let read_long =
            |address: u32| (index < 2).then(|| read_word(address.wrapping_add((index as u32) * 2)));
        let transfer_word = |source, destination| match index {
            0 => Some(read_word(source)),
            1 => Some(write_word(destination, instruction.observations[0])),
            _ => None,
        };
        let rts = |role| {
            if instruction.role == role {
                read_long(sp)
            } else {
                None
            }
        };
        match self.leaf {
            Leaf::Controller1 | Leaf::Controller2 => {
                let (control, data) = if self.leaf == Leaf::Controller1 {
                    (0xa10009, 0xa10003)
                } else {
                    (0xa1000b, 0xa10005)
                };
                match (instruction.role, index) {
                    (0, 0) => Some(write_word(Z80_BUS_REQUEST, 0x0100)),
                    (1, 0) => Some(write_byte(control, 0x40)),
                    (2, 0) => Some(write_byte(data, 0)),
                    (6 | 12, 0) => Some(read_byte(data)),
                    (9, 0) => Some(write_byte(data, 0x40)),
                    (16, 0) => Some(read_byte(Z80_DAC_DELAY)),
                    (16, 1) => Some(write_byte(
                        Z80_DAC_DELAY,
                        u16::from((instruction.observations[0] as u8).wrapping_sub(2)),
                    )),
                    (17, 0) => Some(read_byte(Z80_PSG_ACTIVE)),
                    (17, 1) => Some(write_byte(
                        ram::g_psg_effect_status(self.revision).address(),
                        instruction.observations[0],
                    )),
                    (18, 0) => Some(write_word(Z80_BUS_REQUEST, 0)),
                    _ => rts(19),
                }
            }
            Leaf::WordStream => match instruction.role {
                0 => read_long(sp.wrapping_add(4)),
                1 => read_long(sp.wrapping_add(8)),
                3 => transfer_word(registers.address[0], VDP_DATA),
                _ => rts(5),
            },
            Leaf::Palette => match instruction.role {
                0 => read_long(sp.wrapping_add(4)),
                1 => read_long(sp.wrapping_add(8)),
                6 if index < 2 => Some(write_word(
                    VDP_CONTROL + index as u32 * 2,
                    (registers.data[0] >> (16 - index * 16)) as u16,
                )),
                8 => transfer_word(registers.address[0], VDP_DATA),
                _ => rts(10),
            },
            Leaf::Dma => match instruction.role {
                0 if index < 6 => {
                    let value = registers.data[4 - index / 2];
                    let word = if index & 1 == 0 {
                        value as u16
                    } else {
                        (value >> 16) as u16
                    };
                    Some(write_word(sp.wrapping_sub((index as u32 + 1) * 2), word))
                }
                1 => read_long(sp.wrapping_add(16)),
                2 => read_long(sp.wrapping_add(20)),
                3 => read_long(sp.wrapping_add(24)),
                5 if index == 0 => Some(read_byte(
                    ram::g_mode2_reg_softcopy(self.revision).address(),
                )),
                9 | 13 | 18 | 23 | 29 | 36 if index == 0 => {
                    Some(write_word(VDP_CONTROL, registers.data[0] as u16))
                }
                47 if index < 2 => Some(write_word(
                    ram::g_vdp_dma_command(self.revision)
                        .address()
                        .wrapping_add(index as u32 * 2),
                    (registers.data[0] >> (16 - index * 16)) as u16,
                )),
                48 => transfer_word(ram::g_vdp_dma_command(self.revision).address(), VDP_CONTROL),
                50 => transfer_word(
                    ram::g_vdp_dma_command_low_word(self.revision).address(),
                    VDP_CONTROL,
                ),
                52 if index < 6 => Some(read_word(sp.wrapping_add(index as u32 * 2))),
                52 if index == 6 => Some(read(
                    sp.wrapping_add(12),
                    Width::Word,
                    ReadPurpose::Discarded,
                )),
                _ => rts(53),
            },
        }
    }

    fn commit(&self, instruction: Instruction) -> Commit {
        let role = instruction.role;
        let observed = instruction.observations;
        let long =
            |index: usize| (u32::from(observed[index]) << 16) | u32::from(observed[index + 1]);
        let mut registers = instruction.registers;
        let mut next = role + 1;
        let mut returned = false;
        match self.leaf {
            Leaf::Controller1 | Leaf::Controller2 => match role {
                0 => logical(&mut registers, 0x0100, 0x8000),
                1 | 9 => logical(&mut registers, 0x40, 0x80),
                2 | 18 => logical(&mut registers, 0, 0x8000),
                3 | 10 => assign_word(&mut registers, 1, 10),
                4 | 11 => {
                    if decrement_branch(&mut registers, 1) {
                        next = role;
                    }
                }
                5 => assign_long(&mut registers, 0, 0),
                6 => assign_byte(&mut registers, 0, observed[0] as u8),
                7 => {
                    let byte = registers.data[0] as u8;
                    assign_byte(&mut registers, 0, byte.wrapping_shl(2));
                    carry_extend(&mut registers, byte & 0x40 != 0);
                }
                8 => {
                    let value = registers.data[0] as u8 & 0xc0;
                    assign_byte(&mut registers, 0, value);
                }
                12 => assign_byte(&mut registers, 1, observed[0] as u8),
                13 => {
                    let value = registers.data[1] as u8 & 0x3f;
                    assign_byte(&mut registers, 1, value);
                }
                14 => {
                    let value = registers.data[0] as u8 | registers.data[1] as u8;
                    assign_byte(&mut registers, 0, value);
                }
                15 => {
                    let value = !(registers.data[0] as u8);
                    assign_byte(&mut registers, 0, value);
                }
                16 => {
                    let before = observed[0] as u8;
                    let after = before.wrapping_sub(2);
                    logical(&mut registers, u32::from(after), 0x80);
                    carry_extend(&mut registers, before < 2);
                    if ((before ^ 2) & (before ^ after)) & 0x80 != 0 {
                        registers.status |= 2;
                    }
                }
                17 => logical(&mut registers, u32::from(observed[0]), 0x80),
                19 => returned = true,
                _ => unreachable!("validated controller role"),
            },
            Leaf::WordStream => match role {
                0 => registers.address[0] = long(0),
                1 => assign_long(&mut registers, 0, long(0)),
                2 => subtract_one_word(&mut registers, 0),
                3 => {
                    registers.address[0] = registers.address[0].wrapping_add(2);
                    logical(&mut registers, u32::from(observed[0]), 0x8000);
                }
                4 => {
                    if decrement_branch(&mut registers, 0) {
                        next = 3;
                    }
                }
                5 => returned = true,
                _ => unreachable!("validated word-stream role"),
            },
            Leaf::Palette => match role {
                0 => registers.address[0] = long(0),
                1 => assign_long(&mut registers, 0, long(0)),
                2 => {
                    let value = registers.data[0] & 3;
                    assign_long(&mut registers, 0, value);
                }
                3 => shift_left_long(&mut registers, 0, 5, false),
                4 => {
                    let value = registers.data[0] as u16 | 0xc000;
                    assign_word(&mut registers, 0, value);
                }
                5 => {
                    let value = registers.data[0].rotate_left(16);
                    assign_long(&mut registers, 0, value);
                }
                6 => {
                    let value = registers.data[0];
                    logical(&mut registers, value, 0x8000_0000);
                }
                7 => assign_word(&mut registers, 0, 15),
                8 => {
                    registers.address[0] = registers.address[0].wrapping_add(2);
                    logical(&mut registers, u32::from(observed[0]), 0x8000);
                }
                9 => {
                    if decrement_branch(&mut registers, 0) {
                        next = 8;
                    }
                }
                10 => returned = true,
                _ => unreachable!("validated palette role"),
            },
            Leaf::Dma => match role {
                0 => registers.set_stack_pointer(registers.stack_pointer().wrapping_sub(12)),
                1 => assign_long(&mut registers, 3, long(0)),
                2 => assign_long(&mut registers, 2, long(0)),
                3 => assign_long(&mut registers, 4, long(0)),
                4 => assign_long(&mut registers, 0, 0),
                5 => assign_byte(&mut registers, 0, observed[0] as u8),
                6 => assign_long(&mut registers, 1, 0x10),
                7 => {
                    let value = registers.data[0] | registers.data[1];
                    assign_long(&mut registers, 0, value);
                }
                8 => add_long(&mut registers, 0, 0x8104),
                9 | 13 | 18 | 23 | 29 | 36 => {
                    let value = registers.data[0] as u16;
                    logical(&mut registers, u32::from(value), 0x8000);
                }
                10 | 14 => {
                    let value = registers.data[4];
                    assign_long(&mut registers, 0, value);
                }
                11 | 16 | 21 | 27 => {
                    let value = registers.data[0] & 0xff;
                    assign_long(&mut registers, 0, value);
                }
                12 => add_long(&mut registers, 0, 0x9300),
                15 => shift_right_arithmetic(&mut registers, 0, 8),
                17 => add_long(&mut registers, 0, 0x9400),
                19 | 24 | 30 => {
                    let value = registers.data[3];
                    assign_long(&mut registers, 0, value);
                }
                20 => shift_right_arithmetic(&mut registers, 0, 1),
                22 => add_long(&mut registers, 0, 0x9500),
                25 => assign_long(&mut registers, 1, 9),
                26 | 32 | 40 => {
                    let count = registers.data[1] & 63;
                    shift_right_arithmetic(&mut registers, 0, count);
                }
                28 => add_long(&mut registers, 0, 0x9600),
                31 => assign_long(&mut registers, 1, 17),
                33 => assign_long(&mut registers, 1, 0x7f),
                34 => {
                    let value = registers.data[0] & registers.data[1];
                    assign_long(&mut registers, 0, value);
                }
                35 => add_long(&mut registers, 0, 0x9700),
                37 => {
                    let value = registers.data[2];
                    assign_long(&mut registers, 0, value);
                }
                38 => {
                    let value = registers.data[0] & 0xc000;
                    assign_long(&mut registers, 0, value);
                }
                39 => assign_long(&mut registers, 1, 14),
                41 => {
                    let value = registers.data[2];
                    assign_long(&mut registers, 1, value);
                }
                42 => {
                    let value = registers.data[1] & 0x3fff;
                    assign_long(&mut registers, 1, value);
                }
                43 => assign_long(&mut registers, 2, 16),
                44 => {
                    let count = registers.data[2] & 63;
                    shift_left_long(&mut registers, 1, count, true);
                }
                45 => add_long(&mut registers, 1, 0x4000_0080),
                46 => {
                    let value = registers.data[1];
                    add_long(&mut registers, 0, value);
                }
                47 => {
                    let value = registers.data[0];
                    logical(&mut registers, value, 0x8000_0000);
                }
                48 | 50 => logical(&mut registers, u32::from(observed[0]), 0x8000),
                49 | 51 => {}
                52 => {
                    registers.data[2] = long(0);
                    registers.data[3] = long(2);
                    registers.data[4] = long(4);
                    registers.set_stack_pointer(registers.stack_pointer().wrapping_add(12));
                }
                53 => returned = true,
                _ => unreachable!("validated DMA role"),
            },
        }
        let path = if returned {
            registers.set_stack_pointer(registers.stack_pointer().wrapping_add(4));
            Path::Return {
                address: long(0),
                value: self.leaf.is_controller().then_some(registers.data[0]),
            }
        } else {
            Path::Next(location(self.next.function, next))
        };
        Commit { registers, path }
    }
}

impl TimingSource for Source {
    fn program(&self) -> &'static Program {
        self.leaf.program()
    }

    fn begin_instruction(&mut self, site: InstructionSite, registers: Registers) -> Result<()> {
        if self.poisoned
            || self.finished
            || self.instruction.is_some()
            || site.location != self.next
        {
            return Err(self.reject("platform source instruction is out of sequence"));
        }
        self.instruction = Some(Instruction {
            role: site.location.role,
            registers,
            acknowledged: 0,
            observations: [0; 7],
            pending: None,
        });
        Ok(())
    }

    fn next_effect(&mut self) -> Result<Option<Effect>> {
        if self.poisoned {
            return Err(self.reject("platform source protocol is poisoned"));
        }
        let Some(instruction) = self.instruction.as_ref() else {
            return Err(self.reject("platform source has no active instruction"));
        };
        if instruction.pending.is_some() {
            return Err(self.reject("platform source effect was not acknowledged"));
        }
        let effect = self.effect(instruction);
        self.instruction
            .as_mut()
            .expect("checked active instruction")
            .pending = effect;
        Ok(effect)
    }

    fn acknowledge(&mut self, actual: Observation) -> Result<()> {
        if self.poisoned {
            return Err(self.reject("platform source protocol is poisoned"));
        }
        let Some(instruction) = self.instruction.as_mut() else {
            return Err(self.reject("platform source acknowledgement has no instruction"));
        };
        if instruction.pending != Some(actual.effect)
            || instruction.acknowledged >= instruction.observations.len()
            || (actual.effect.width == Width::Byte && actual.value > 0xff)
            || matches!(actual.effect.kind, EffectKind::Write(value) if value != actual.value)
        {
            return Err(self.reject("platform source acknowledgement differs from its effect"));
        }
        instruction.observations[instruction.acknowledged] = actual.value;
        instruction.acknowledged += 1;
        instruction.pending = None;
        Ok(())
    }

    fn finish_instruction(&mut self) -> Result<Commit> {
        if self.poisoned {
            return Err(self.reject("platform source protocol is poisoned"));
        }
        let Some(instruction) = self.instruction.take() else {
            return Err(self.reject("platform source finish has no instruction"));
        };
        if instruction.pending.is_some() || self.effect(&instruction).is_some() {
            return Err(self.reject("platform source instruction has missing acknowledgements"));
        }
        let commit = self.commit(instruction);
        match commit.path {
            Path::Next(next) => self.next = next,
            Path::Return { .. } => self.finished = true,
        }
        Ok(commit)
    }
}

fn logical(registers: &mut Registers, value: u32, sign: u32) {
    let nz = if value == 0 {
        4
    } else if value & sign != 0 {
        8
    } else {
        0
    };
    registers.status = (registers.status & !0xf) | nz;
}

fn assign_long(registers: &mut Registers, register: usize, value: u32) {
    registers.data[register] = value;
    logical(registers, value, 0x8000_0000);
}

fn assign_word(registers: &mut Registers, register: usize, value: u16) {
    registers.data[register] = (registers.data[register] & 0xffff_0000) | u32::from(value);
    logical(registers, u32::from(value), 0x8000);
}

fn assign_byte(registers: &mut Registers, register: usize, value: u8) {
    registers.data[register] = (registers.data[register] & 0xffff_ff00) | u32::from(value);
    logical(registers, u32::from(value), 0x80);
}

fn carry_extend(registers: &mut Registers, carry: bool) {
    registers.status = (registers.status & !0x11) | if carry { 0x11 } else { 0 };
}

fn decrement_branch(registers: &mut Registers, register: usize) -> bool {
    let low = (registers.data[register] as u16).wrapping_sub(1);
    registers.data[register] = (registers.data[register] & 0xffff_0000) | u32::from(low);
    low != 0xffff
}

fn subtract_one_word(registers: &mut Registers, register: usize) {
    let before = registers.data[register] as u16;
    assign_word(registers, register, before.wrapping_sub(1));
    carry_extend(registers, before == 0);
    if before == 0x8000 {
        registers.status |= 2;
    }
}

fn add_long(registers: &mut Registers, register: usize, value: u32) {
    let before = registers.data[register];
    let (after, carry) = before.overflowing_add(value);
    assign_long(registers, register, after);
    carry_extend(registers, carry);
    if (!(before ^ value) & (before ^ after)) & 0x8000_0000 != 0 {
        registers.status |= 2;
    }
}

fn shift_right_arithmetic(registers: &mut Registers, register: usize, count: u32) {
    let before = registers.data[register];
    let after = if count < 32 {
        ((before as i32) >> count) as u32
    } else {
        ((before as i32) >> 31) as u32
    };
    assign_long(registers, register, after);
    if count != 0 {
        let carry = if count < 32 {
            before >> (count - 1) & 1 != 0
        } else {
            before & 0x8000_0000 != 0
        };
        carry_extend(registers, carry);
    }
}

fn shift_left_long(registers: &mut Registers, register: usize, count: u32, arithmetic: bool) {
    let before = registers.data[register];
    let after = if count < 32 { before << count } else { 0 };
    assign_long(registers, register, after);
    if count != 0 {
        carry_extend(registers, count <= 32 && before >> (32 - count) & 1 != 0);
        let overflow = if count >= 32 {
            before != 0
        } else {
            let high = (before as i32) >> (31 - count);
            high != 0 && high != -1
        };
        if arithmetic && overflow {
            registers.status |= 2;
        }
    }
}

fn execute(context: &mut dyn Context, leaf: Leaf) -> Result<Option<u32>> {
    let mut source = Source::new(context.revision(), leaf);
    let exit = context.execute_timing_program(source.program().entry.function, &mut source)?;
    Ok(exit.value)
}

/// G/src/asm/controllers.s:12 (`GetCtrl1Input`), actual instruction-role entry.
pub fn get_ctrl1_input(context: &mut dyn Context) -> Result<Option<u32>> {
    execute(context, Leaf::Controller1)
}

/// G/src/asm/controllers.s:40 (`GetCtrl2Input`), actual instruction-role entry.
pub fn get_ctrl2_input(context: &mut dyn Context) -> Result<Option<u32>> {
    execute(context, Leaf::Controller2)
}

/// G/src/asm/vdp.s:103 (`MoveDataToVRAM`), actual argument and data bus reads.
pub fn move_data_to_vram(context: &mut dyn Context) -> Result<Option<u32>> {
    execute(context, Leaf::WordStream)
}

/// G/src/asm/vdp.s:153 (`CopyPaletteToCRAMSlot`), actual command/data bus effects.
pub fn copy_palette_to_cram_slot(context: &mut dyn Context) -> Result<Option<u32>> {
    execute(context, Leaf::Palette)
}

/// G/src/asm/dma.s:15 (`DmaCopyWordsToVram`), real MOVEM/arguments/DMA/RTS path.
pub fn dma_copy_words_to_vram(context: &mut dyn Context) -> Result<Option<u32>> {
    execute(context, Leaf::Dma)
}

#[cfg(test)]
#[path = "timed_tests.rs"]
mod tests;
