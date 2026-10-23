use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tje_rom::Revision;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionRange {
    pub name: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone)]
pub struct Catalogue {
    functions: Vec<FunctionRange>,
}

impl Catalogue {
    pub fn committed(revision: Revision) -> Self {
        Self::new(
            tje_game::functions()
                .map(|function| FunctionRange {
                    name: function.name.into(),
                    start: function.address(revision),
                    end: function.address(revision) + function.size(revision),
                })
                .collect(),
        )
    }

    pub fn new(mut functions: Vec<FunctionRange>) -> Self {
        functions.sort_by_key(|function| function.start);
        Self { functions }
    }

    pub fn function_at(&self, pc: u32) -> Option<&FunctionRange> {
        let upper = self
            .functions
            .partition_point(|function| function.start <= pc);
        self.functions[..upper]
            .iter()
            .rev()
            .find(|function| pc < function.end)
    }

    pub fn name(&self, pc: u32) -> &str {
        self.function_at(pc)
            .map_or("<uncatalogued>", |function| &function.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionCoverage {
    pub pc: u32,
    pub function: String,
    pub executions: u64,
    pub first_frame: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCoverage {
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub executions: u64,
    pub distinct_instructions: usize,
    pub first_frame: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub instructions_executed: u64,
    pub instructions: Vec<InstructionCoverage>,
    pub functions: Vec<FunctionCoverage>,
    /// Catalogue functions with no observed instruction, not assumed tested.
    pub unreached_functions: Vec<FunctionRange>,
    pub uncatalogued_instructions: usize,
}

#[derive(Default)]
pub(crate) struct CoverageCollector {
    instructions: HashMap<u32, (u64, u64)>,
    total: u64,
}

impl CoverageCollector {
    pub fn record(&mut self, pc: u32, frame: u64) {
        self.total += 1;
        self.instructions
            .entry(pc)
            .and_modify(|(count, _)| *count += 1)
            .or_insert((1, frame));
    }

    pub fn finish(self, catalogue: &Catalogue) -> CoverageReport {
        let mut report = CoverageReport {
            instructions_executed: self.total,
            ..CoverageReport::default()
        };
        let mut functions = BTreeMap::<u32, FunctionCoverage>::new();
        for (pc, (executions, first_frame)) in self.instructions {
            let function = catalogue.function_at(pc);
            if let Some(function) = function {
                let entry = functions
                    .entry(function.start)
                    .or_insert_with(|| FunctionCoverage {
                        name: function.name.clone(),
                        start: function.start,
                        end: function.end,
                        executions: 0,
                        distinct_instructions: 0,
                        first_frame,
                    });
                entry.executions += executions;
                entry.distinct_instructions += 1;
                entry.first_frame = entry.first_frame.min(first_frame);
            } else {
                report.uncatalogued_instructions += 1;
            }
            report.instructions.push(InstructionCoverage {
                pc,
                function: function
                    .map_or("<uncatalogued>", |function| &function.name)
                    .into(),
                executions,
                first_frame,
            });
        }
        report.unreached_functions = catalogue
            .functions
            .iter()
            .filter(|function| !functions.contains_key(&function.start))
            .cloned()
            .collect();
        report
            .instructions
            .sort_by_key(|instruction| instruction.pc);
        report.functions = functions.into_values().collect();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_not_entries_define_function_coverage() {
        let catalogue = Catalogue::new(vec![
            FunctionRange {
                name: "Caller".into(),
                start: 0x100,
                end: 0x110,
            },
            FunctionRange {
                name: "Unreached".into(),
                start: 0x120,
                end: 0x130,
            },
        ]);
        let mut collector = CoverageCollector::default();
        collector.record(0x104, 3);
        collector.record(0x104, 4);
        collector.record(0x108, 4);
        collector.record(0x110, 5);
        let report = collector.finish(&catalogue);
        assert_eq!(report.instructions_executed, 4);
        assert_eq!(report.instructions.len(), 3);
        assert_eq!(report.functions[0].executions, 3);
        assert_eq!(report.functions[0].distinct_instructions, 2);
        assert_eq!(report.functions[0].first_frame, 3);
        assert_eq!(report.unreached_functions[0].name, "Unreached");
        assert_eq!(report.uncatalogued_instructions, 1);
        assert_eq!(catalogue.name(0x110), "<uncatalogued>");
    }
}
