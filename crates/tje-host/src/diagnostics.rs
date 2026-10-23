//! Bounded failure evidence, captured before check forks and their host disappear.
//! These observations never select or relax a comparison obligation.
use crate::{Host, hooks::Hook};
use genesis_components::vdp::debug::{VdpStateDifference, VdpStateDifferences};
use genesis_core::GenesisEmulator;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use tje_game::execution::ExecutionClass;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VdpMismatchOrigin {
    Check,
    StackProbe {
        side: &'static str,
        xor_mask: u8,
        selected_address: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VdpComparisonScope {
    Logical,
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct VdpMismatchReport {
    pub frame: u64,
    pub function: String,
    pub scope: VdpComparisonScope,
    pub origin: VdpMismatchOrigin,
    #[serde(serialize_with = "serialize_state_differences")]
    pub state: VdpStateDifferences,
}

#[derive(Serialize)]
struct Difference<'a> {
    component: &'static str,
    field: &'static str,
    registers: &'static [u8],
    index: Option<usize>,
    address: Option<u32>,
    original: &'a str,
    translated: &'a str,
}

struct Differences<'a>(&'a [VdpStateDifference]);
impl Serialize for Differences<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|difference| {
            let location = difference.location;
            Difference {
                component: location.component,
                field: location.field,
                registers: location.registers,
                index: location.index,
                address: location.address,
                original: &difference.original,
                translated: &difference.translated,
            }
        }))
    }
}

fn serialize_state_differences<S: Serializer>(
    state: &VdpStateDifferences,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut output = serializer.serialize_struct("VdpStateDifferences", 2)?;
    output.serialize_field("differences", &Differences(&state.differences))?;
    output.serialize_field("truncated", &state.truncated)?;
    output.end()
}

impl VdpMismatchReport {
    fn first_difference(&self) -> String {
        let Some(first) = self.state.differences.first() else {
            return "no differing field identified by the diagnostic walker".into();
        };
        let location = first.location;
        let mut detail = format!("{}.{}", location.component, location.field);
        if !location.registers.is_empty() {
            detail.push_str(&format!(" registers={:?}", location.registers));
        }
        if let Some(address) = location.address {
            detail.push_str(&format!(" address=0x{address:04X}"));
        } else if let Some(index) = location.index {
            detail.push_str(&format!(" index={index}"));
        }
        detail.push_str(&format!(
            ": original={}, Rust={} ({} retained difference(s){})",
            first.original,
            first.translated,
            self.state.differences.len(),
            if self.state.truncated {
                ", further differences truncated"
            } else {
                ""
            },
        ));
        detail
    }
}

impl Host {
    /// Enrich the existing VDP failure only; never diagnose a different error as
    /// its cause or compare timing-only fields under a logical contract.
    pub(super) fn record_vdp_mismatch(
        &mut self,
        detail: &mut String,
        original: &GenesisEmulator,
        translated: &GenesisEmulator,
        hook: &Hook,
        origin: VdpMismatchOrigin,
    ) {
        if !detail.starts_with("VDP state differs") {
            return;
        }
        let logical = hook.execution_class == ExecutionClass::TimeAware;
        let report = VdpMismatchReport {
            frame: self.frame,
            function: hook.symbol.name.to_owned(),
            scope: if logical {
                VdpComparisonScope::Logical
            } else {
                VdpComparisonScope::Full
            },
            origin,
            state: original
                .hybrid_bus()
                .vdp
                .hybrid_state_differences(&translated.hybrid_bus().vdp, logical),
        };
        detail.push_str("; first difference: ");
        detail.push_str(&report.first_difference());
        self.checks.last_vdp_mismatch = Some(report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genesis_config::GenesisEmulatorConfig;
    use genesis_core::GenesisBus;
    use jgenesis_common::{
        debug::DebugMemoryView,
        frontend::{PartialClone, TimingMode},
    };
    use m68000_emu::BusInterface;

    fn bus() -> GenesisBus {
        GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default())
    }

    #[test]
    fn reports_register_indices_latches_dma_and_each_memory_address_without_mutation() {
        let original = bus();
        let mut changed = original.partial_clone();
        changed.vdp.write_control(0x8aff);
        let report = original.vdp.hybrid_state_differences(&changed.vdp, true);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.location.registers == [10]
                    && difference.location.field == "h_interrupt_interval"
                    && difference.original == "0"
                    && difference.translated == "255")
        );
        assert!(!original.vdp.hybrid_logic_state_eq(&changed.vdp));
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.location.component == "control_port")
        );
        let mut changed = original.partial_clone();
        changed.vdp.write_control(0x4000); // Unfinished command is a real latch output.
        let report = original.vdp.hybrid_state_differences(&changed.vdp, true);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.location.field == "write_flag"
                    && difference.original == "First"
                    && difference.translated == "Second")
        );
        let mut changed = original.partial_clone();
        changed.vdp.write_control(0x9307);
        let report = original.vdp.hybrid_state_differences(&changed.vdp, true);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.location.field == "dma_length"
                    && difference.location.registers == [19, 20]
                    && difference.original == "0"
                    && difference.translated == "7")
        );
        for (memory, address) in [("vram", 17), ("cram", 6), ("vsram", 10)] {
            let mut changed = original.partial_clone();
            match memory {
                "vram" => changed.vdp.debug_vram_view().write(address, 0xab),
                "cram" => changed.vdp.debug_cram_view().write(address, 0xab),
                "vsram" => changed.vdp.debug_vsram_view().write(address, 0xab),
                _ => unreachable!(),
            }
            let before = bincode::encode_to_vec(&changed.vdp, bincode::config::standard()).unwrap();
            let report = original.vdp.hybrid_state_differences(&changed.vdp, true);
            let first = &report.differences[0];
            assert_eq!(first.location.component, memory);
            assert_eq!(first.location.address, Some(address as u32));
            assert_ne!(first.original, first.translated);
            assert!(!report.truncated);
            assert_eq!(
                before,
                bincode::encode_to_vec(&changed.vdp, bincode::config::standard()).unwrap()
            );
        }
    }

    #[test]
    fn logical_scope_excludes_fifo_phase_but_full_scope_describes_it() {
        let mut pending = bus();
        for value in [0x8f02, 0x4000, 0] {
            pending.write_word(0xc00004, value);
            pending.apply_pending_writes();
        }
        pending.write_word(0xc00000, 0);
        pending.apply_pending_writes();
        let mut completed = pending.partial_clone();
        completed.hybrid_set_functional_mode(true);
        assert!(pending.vdp.hybrid_logic_state_eq(&completed.vdp));
        assert!(!pending.vdp.hybrid_state_eq(&completed.vdp));
        assert!(
            pending
                .vdp
                .hybrid_state_differences(&completed.vdp, true)
                .differences
                .is_empty()
        );
        let report = pending.vdp.hybrid_state_differences(&completed.vdp, false);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.location.component == "fifo")
        );
        assert!(report.differences.iter().all(
            |difference| difference.original.len() <= 268 && difference.translated.len() <= 268
        ));
    }

    #[test]
    fn many_memory_differences_are_ordered_and_bounded() {
        let original = bus();
        let mut changed = original.partial_clone();
        for index in 0..100 {
            changed.vdp.debug_vram_view().write(index, 0xff);
        }
        let state = original.vdp.hybrid_state_differences(&changed.vdp, true);
        assert_eq!(state.differences.len(), 16);
        assert!(state.truncated);
        for (index, difference) in state.differences.iter().enumerate() {
            assert_eq!(difference.location.address, Some(index as u32));
        }
        let report = VdpMismatchReport {
            frame: 42,
            function: "AuthoredVdpMismatch".into(),
            scope: VdpComparisonScope::Logical,
            origin: VdpMismatchOrigin::Check,
            state,
        };
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["state"]["differences"].as_array().unwrap().len(), 16);
        assert_eq!(value["state"]["differences"][0]["address"], 0);
        assert_eq!(value["state"]["differences"][0]["original"], "0");
        assert_eq!(value["state"]["differences"][0]["translated"], "255");
        assert!(report.first_difference().contains("address=0x0000"));
    }
}
