//! Source-metadata table SHAPES only; offsets and targets stay in the user ROM.
//! Inventory d60266bebb8ef9d6db77d212e82061acb2628fa093ab626001f7949e3f7134e4.
//! MCC68K switch words relocate from the table start. An indexed-load PC base
//! is not the relocation base; those differ for several compiler layouts.
use super::{JumpTable, Revision};
pub(super) fn tables(revision: Revision) -> Vec<JumpTable> {
    let index = match revision {
        Revision::Rev02 => 0,
        Revision::Rev00 => 1,
    };
    SHAPES
        .iter()
        .map(|s| JumpTable {
            jump_pc: s.jump_pc[index],
            address: s.address[index],
            count: s.count,
            relative_base: s.address[index],
        })
        .collect()
}
struct Shape {
    jump_pc: [u32; 2],
    address: [u32; 2],
    count: u16,
}
const SHAPES: &[Shape] = &[
    Shape {
        jump_pc: [0x009E98, 0x009E78],
        address: [0x009E9C, 0x009E7C],
        count: 6,
    }, // T01 TextMapCharacterToGlyph
    Shape {
        jump_pc: [0x009EBE, 0x009E9E],
        address: [0x009EC2, 0x009EA2],
        count: 14,
    }, // T02 TextMapCharacterToGlyph
    Shape {
        jump_pc: [0x01734E, 0x01731E],
        address: [0x017352, 0x017322],
        count: 8,
    }, // T03 SetupSelectedPresent
    Shape {
        jump_pc: [0x019C7A, 0x019C38],
        address: [0x019C7E, 0x019C3C],
        count: 4,
    }, // T04 AI_GenericEnemyThink
    Shape {
        jump_pc: [0x01A278, 0x01A236],
        address: [0x01A27C, 0x01A23A],
        count: 4,
    }, // T05 EarthlingThinkTimedMovement
    Shape {
        jump_pc: [0x01DDEE, 0x01DDAC],
        address: [0x01DDF2, 0x01DDB0],
        count: 4,
    }, // T06 GamePaused
    Shape {
        jump_pc: [0x02032C, 0x0202B4],
        address: [0x020330, 0x0202B8],
        count: 10,
    }, // T07 ShowShipPieceName
    Shape {
        jump_pc: [0x022574, 0x0224FC],
        address: [0x022578, 0x022500],
        count: 4,
    }, // T08 SantaLogic
    Shape {
        jump_pc: [0x02350A, 0x023492],
        address: [0x02350E, 0x023496],
        count: 5,
    }, // T09 NewGameMainMenu
    Shape {
        jump_pc: [0x0296B6, 0x029648],
        address: [0x0296BA, 0x02964C],
        count: 5,
    }, // T10 IntroUpdateFlyingEntityMotion
    Shape {
        jump_pc: [0x029DBA, 0x029D4C],
        address: [0x029DBE, 0x029D50],
        count: 39,
    }, // T11 IntroPart1
    Shape {
        jump_pc: [0x02A3EA, 0x02A37C],
        address: [0x02A3EE, 0x02A380],
        count: 10,
    }, // T12 IntroPart2
    Shape {
        jump_pc: [0x02A688, 0x02A61A],
        address: [0x02A68C, 0x02A61E],
        count: 8,
    }, // T13 IntroPart3
    Shape {
        jump_pc: [0x02A99C, 0x02A92E],
        address: [0x02A9A0, 0x02A932],
        count: 5,
    }, // T14 IntroPart4
    Shape {
        jump_pc: [0x02AC50, 0x02ABE2],
        address: [0x02AC54, 0x02ABE6],
        count: 11,
    }, // T15 IntroPart5
    Shape {
        jump_pc: [0x02AEB0, 0x02AE42],
        address: [0x02AEB4, 0x02AE46],
        count: 10,
    }, // T16 IntroPart6
    Shape {
        jump_pc: [0x02B186, 0x02B118],
        address: [0x02B18A, 0x02B11C],
        count: 7,
    }, // T17 IntroPart7
    Shape {
        jump_pc: [0x02B672, 0x02B604],
        address: [0x02B676, 0x02B608],
        count: 6,
    }, // T18 IntroPart10
    Shape {
        jump_pc: [0x02B896, 0x02B828],
        address: [0x02B89A, 0x02B82C],
        count: 32,
    }, // T19 IntroPart11
    Shape {
        jump_pc: [0x0376F6, 0x037688],
        address: [0x0376FA, 0x03768C],
        count: 13,
    }, // T20 EndingPlayDialogueAndDeparture
    Shape {
        jump_pc: [0x037AC8, 0x037A5A],
        address: [0x037ACC, 0x037A5E],
        count: 4,
    }, // T21 EndingPlayUpwardRocketshipDeparture
    Shape {
        jump_pc: [0x037D52, 0x037CE4],
        address: [0x037D56, 0x037CE8],
        count: 18,
    }, // T22 EndingPlayScrollingCredits
    Shape {
        jump_pc: [0x038172, 0x038104],
        address: [0x038176, 0x038108],
        count: 4,
    }, // T23 EndingPlayDescendingRocketshipScene
    Shape {
        jump_pc: [0x03B47C, 0x03B40C],
        address: [0x03B480, 0x03B410],
        count: 6,
    }, // T24 JamOutUpdateLayoutsAndScroll
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocation_is_relative_to_table_not_indexed_load_pc() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            for table in tables(revision) {
                assert_eq!(table.relative_base, table.address);
                assert_eq!(table.address, table.jump_pc + 4);
            }
        }
    }
}
