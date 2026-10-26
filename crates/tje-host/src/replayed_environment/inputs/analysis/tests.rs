//! Authored chronology primitive control; no body, invocation or proof receipt
//! is minted from these events. The Analyzer owner is an actual synthetic call.
use super::*;
use crate::replayed_environment::tests::causal_inputs;
use tje_mem::Revision;

#[test]
fn same_instruction_reads_use_physical_commit_and_z80_order_not_issue_order() {
    let (_host, original) = causal_inputs::capture(Revision::Rev02, vec![0x4e75], vec![]);
    let mut analyzer = Analyzer::new(&original);
    let accesses = [
        Access {
            address: 0xff0010,
            word: true,
            direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
            value: 0x1234,
        },
        Access {
            address: 0xff0010,
            word: true,
            direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
            value: 0x5678,
        },
        Access {
            address: 0xff0010,
            word: true,
            direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
            value: 0x9abc,
        },
    ];
    for (index, access) in accesses.into_iter().enumerate() {
        let occurrence = analyzer.origins.new_read(access, 0xffff).unwrap();
        analyzer.slots.push(ReadSlot {
            issue: index + 1,
            occurrence,
            access,
            mandatory: false,
        });
    }
    let mut sources = [Origin::EMPTY; 16];
    for (bit, source) in sources.iter_mut().enumerate() {
        *source = analyzer
            .origins
            .registers(StateMask::single(StateField::Data(2), 1 << bit));
    }
    let commit = Access {
        address: 0xff0010,
        word: true,
        direction: Direction::Write,
        value: 0x5678,
    };
    // Store issue0 already exists. Its actual physical commit is later than
    // read issue1; a Z80 overwrite is between read issues2 and3.
    analyzer.pending.push(Store {
        issue: 0,
        access: commit,
        sources,
        committed: false,
    });
    analyzer
        .chronology(&[
            Chronology::CpuRead {
                access: accesses[0],
                issue: 1,
            },
            Chronology::CpuCommit {
                access: commit,
                issue: 0,
            },
            Chronology::CpuRead {
                access: accesses[1],
                issue: 2,
            },
            Chronology::Z80Write(Access {
                value: 0x9abc,
                ..commit
            }),
            Chronology::CpuRead {
                access: accesses[2],
                issue: 3,
            },
        ])
        .unwrap();
    let roots: Vec<_> = analyzer.slots.iter().map(|slot| slot.occurrence).collect();
    let before = analyzer.origins.read_source(roots[0], 0xffff).unwrap();
    let after = analyzer.origins.read_source(roots[1], 0xffff).unwrap();
    let z80 = analyzer.origins.read_source(roots[2], 0xffff).unwrap();
    let before = analyzer.origins.demand(before).unwrap();
    assert_eq!(before.memory[&0xff0010].value, 0x12);
    assert_eq!(before.memory[&0xff0011].value, 0x34);
    assert!(before.registers.is_empty());
    assert_eq!(
        analyzer.origins.demand(after).unwrap().registers,
        StateMask::single(StateField::Data(2), 0xffff)
    );
    let z80 = analyzer.origins.demand(z80).unwrap();
    assert!(z80.registers.is_empty() && z80.memory.is_empty());
    assert!(analyzer.pending[0].committed);
}
