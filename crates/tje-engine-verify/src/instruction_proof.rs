//! Strict, allocation-free success path for original-instruction equivalence.
//!
//! Starting from equal memory, equal ordered *actual commits* from CPU and Z80
//! establish equal RAM inductively. CPU issued reads/writes are compared too,
//! including ROM/prefetch/discarded accesses. Full frame/device snapshots remain
//! a separate required proof.
use crate::{Difference, Machine};
use genesis_core::native::RamWrite;
use serde::{Deserialize, Serialize};
use tje_engine_core::CpuState;
use tje_engine_runtime::BusAccess;

/// Borrowed journals are cleared/reused by the adapters, not allocated per step.
#[derive(Debug, Clone, Copy)]
pub struct InstructionProof<'a> {
    pub cpu: [u32; 25],
    pub base_cycles: u32,
    pub accesses: &'a [BusAccess],
    pub ram_writes: &'a [RamWrite],
}

/// Available only in the development verification crate. A production backend
/// never receives an opcode-interpreter fallback from this interface.
pub trait InstructionMachine: Machine {
    fn enable_instruction_proof(&mut self);
    fn instruction_proof(&self) -> InstructionProof<'_>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionProofCounts {
    pub scheduler_steps_compared: u64,
    pub instructions_compared: u64,
    pub bus_accesses_compared: u64,
    pub ram_writes_compared: u64,
    pub initial_state_compared: bool,
}

pub fn cpu_state_words(cpu: &CpuState) -> [u32; 25] {
    let mut words = [0; 25];
    words[..8].copy_from_slice(&cpu.d);
    words[8..15].copy_from_slice(&cpu.a);
    words[15..].copy_from_slice(&[
        cpu.usp,
        cpu.ssp,
        cpu.pc,
        cpu.sr,
        cpu.prefetch,
        cpu.pending_interrupt,
        cpu.stopped,
        cpu.frozen,
        cpu.address_error,
        cpu.last_muldiv,
    ]);
    words
}

struct CpuField {
    state: tje_engine_core::ir::StateField,
    name: &'static str,
}

const CPU_FIELDS: [CpuField; 25] = {
    use tje_engine_core::ir::StateField::*;
    [
        CpuField {
            state: Data(0),
            name: "d0",
        },
        CpuField {
            state: Data(1),
            name: "d1",
        },
        CpuField {
            state: Data(2),
            name: "d2",
        },
        CpuField {
            state: Data(3),
            name: "d3",
        },
        CpuField {
            state: Data(4),
            name: "d4",
        },
        CpuField {
            state: Data(5),
            name: "d5",
        },
        CpuField {
            state: Data(6),
            name: "d6",
        },
        CpuField {
            state: Data(7),
            name: "d7",
        },
        CpuField {
            state: Address(0),
            name: "a0",
        },
        CpuField {
            state: Address(1),
            name: "a1",
        },
        CpuField {
            state: Address(2),
            name: "a2",
        },
        CpuField {
            state: Address(3),
            name: "a3",
        },
        CpuField {
            state: Address(4),
            name: "a4",
        },
        CpuField {
            state: Address(5),
            name: "a5",
        },
        CpuField {
            state: Address(6),
            name: "a6",
        },
        CpuField {
            state: Usp,
            name: "usp",
        },
        CpuField {
            state: Ssp,
            name: "ssp",
        },
        CpuField {
            state: Pc,
            name: "pc",
        },
        CpuField {
            state: Sr,
            name: "sr",
        },
        CpuField {
            state: Prefetch,
            name: "prefetch",
        },
        CpuField {
            state: PendingInterrupt,
            name: "pending_interrupt",
        },
        CpuField {
            state: Stopped,
            name: "stopped",
        },
        CpuField {
            state: Frozen,
            name: "frozen",
        },
        CpuField {
            state: AddressError,
            name: "address_error",
        },
        CpuField {
            state: LastMulDiv,
            name: "last_muldiv",
        },
    ]
};

pub fn compare_instruction_proofs(
    expected: &InstructionProof<'_>,
    actual: &InstructionProof<'_>,
) -> Option<Difference> {
    if expected.cpu != actual.cpu {
        let index = expected
            .cpu
            .iter()
            .zip(actual.cpu)
            .position(|(&a, b)| a != b)
            .expect("unequal fixed-size arrays have a differing element");
        let field = &CPU_FIELDS[index];
        debug_assert_eq!(field.state.index() as usize, index);
        return Some(Difference {
            domain: format!("instruction_cpu.{}", field.name),
            index: Some(index),
            reference: format!("{:08X}", expected.cpu[index]),
            candidate: format!("{:08X}", actual.cpu[index]),
        });
    }
    if expected.base_cycles != actual.base_cycles {
        return Some(Difference {
            domain: "instruction_base_cycles".into(),
            index: None,
            reference: expected.base_cycles.to_string(),
            candidate: actual.base_cycles.to_string(),
        });
    }
    // Slice equality is the no-allocation common path. On failure, preserve the
    // first ordered difference rather than comparing sets or dropping prefetch.
    if expected.accesses != actual.accesses {
        let index = expected
            .accesses
            .iter()
            .zip(actual.accesses)
            .position(|(a, b)| a != b)
            .unwrap_or(expected.accesses.len().min(actual.accesses.len()));
        return Some(Difference {
            domain: "instruction_bus_access".into(),
            index: Some(index),
            reference: expected
                .accesses
                .get(index)
                .map_or_else(|| "<end>".into(), |event| format!("{event:?}")),
            candidate: actual
                .accesses
                .get(index)
                .map_or_else(|| "<end>".into(), |event| format!("{event:?}")),
        });
    }
    if expected.ram_writes != actual.ram_writes {
        let index = expected
            .ram_writes
            .iter()
            .zip(actual.ram_writes)
            .position(|(a, b)| a != b)
            .unwrap_or(expected.ram_writes.len().min(actual.ram_writes.len()));
        return Some(Difference {
            domain: "instruction_ram_write".into(),
            index: Some(index),
            reference: expected
                .ram_writes
                .get(index)
                .map_or_else(|| "<end>".into(), |event| format!("{event:?}")),
            candidate: actual
                .ram_writes
                .get(index)
                .map_or_else(|| "<end>".into(), |event| format!("{event:?}")),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_engine_core::ReadKind;

    #[test]
    fn every_architectural_word_is_compared() {
        let cpu = cpu_state_words(&CpuState::default());
        let expected = InstructionProof {
            cpu,
            base_cycles: 4,
            accesses: &[],
            ram_writes: &[],
        };
        assert!(compare_instruction_proofs(&expected, &expected).is_none());
        for (index, field) in CPU_FIELDS.iter().enumerate() {
            assert_eq!(field.state.index() as usize, index);
            let mut actual = expected;
            actual.cpu[index] = 1;
            let difference = compare_instruction_proofs(&expected, &actual).unwrap();
            assert_eq!(difference.index, Some(index));
            assert!(difference.domain.ends_with(field.name));
        }
    }

    #[test]
    fn charge_read_kind_width_order_and_missing_access_are_not_projected() {
        let semantic = BusAccess::Read {
            address: 0,
            word: true,
            value: 7,
            kind: ReadKind::Semantic,
        };
        let prefetch = BusAccess::Read {
            address: 0,
            word: true,
            value: 7,
            kind: ReadKind::Prefetch,
        };
        let write = BusAccess::Write {
            address: 0xFF0000,
            word: true,
            value: 7,
        };
        let events = [semantic, write];
        let expected = InstructionProof {
            cpu: [0; 25],
            base_cycles: 12,
            accesses: &events,
            ram_writes: &[],
        };
        let mut actual = expected;
        actual.base_cycles += 1;
        assert_eq!(
            compare_instruction_proofs(&expected, &actual)
                .unwrap()
                .domain,
            "instruction_base_cycles"
        );
        for changed in [
            vec![prefetch, write],
            vec![write, semantic],
            vec![semantic],
            vec![
                BusAccess::Read {
                    address: 0,
                    word: false,
                    value: 7,
                    kind: ReadKind::Semantic,
                },
                write,
            ],
            vec![
                semantic,
                BusAccess::Write {
                    address: 0xFF0000,
                    word: true,
                    value: 8,
                },
            ],
            vec![semantic, write, write],
        ] {
            let actual = InstructionProof {
                accesses: &changed,
                ..expected
            };
            assert_eq!(
                compare_instruction_proofs(&expected, &actual)
                    .unwrap()
                    .domain,
                "instruction_bus_access"
            );
        }
    }

    #[test]
    fn actual_ram_commits_compare_all_origins_in_order() {
        let writes = [
            RamWrite {
                address: 0xFF0000,
                word: true,
                value: 7,
            },
            RamWrite {
                address: 0xFF0003,
                word: false,
                value: 8,
            },
        ];
        let expected = InstructionProof {
            cpu: [0; 25],
            base_cycles: 4,
            accesses: &[],
            ram_writes: &writes,
        };
        assert!(compare_instruction_proofs(&expected, &expected).is_none());
        for changed in [
            vec![writes[1], writes[0]],
            vec![writes[0]],
            vec![
                RamWrite {
                    value: 9,
                    ..writes[0]
                },
                writes[1],
            ],
            vec![
                RamWrite {
                    word: false,
                    ..writes[0]
                },
                writes[1],
            ],
        ] {
            let actual = InstructionProof {
                ram_writes: &changed,
                ..expected
            };
            assert_eq!(
                compare_instruction_proofs(&expected, &actual)
                    .unwrap()
                    .domain,
                "instruction_ram_write",
            );
        }
    }
}
