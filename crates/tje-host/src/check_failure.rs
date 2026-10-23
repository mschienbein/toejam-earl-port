//! Explicit corpus-only failure collection. The live machine still executes
//! original instructions exactly once; checked forks never become replacements.
use crate::{Host, HostError, Mode, Result, diagnostics::VdpMismatchReport, hooks::Hook};
use serde::Serialize;

const FAILURE_DETAIL_LIMIT: usize = 4096;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckFailurePolicy {
    #[default]
    FailFast,
    ContinueOriginal,
}

#[derive(Debug, Serialize)]
pub struct CheckFailure {
    pub frame: u64,
    pub function: String,
    pub entry_pc: u32,
    pub master_clock: u64,
    /// Position in this live frame's ordered hardware-write stream at entry.
    pub frame_write_index: usize,
    /// First differing owned-effect/device-access ordinal, when the error names it.
    pub trace_index: Option<usize>,
    pub detail: String,
    pub detail_truncated: bool,
    pub vdp_mismatch: Option<VdpMismatchReport>,
}

fn bounded_detail(mut detail: String) -> (String, bool) {
    if detail.len() <= FAILURE_DETAIL_LIMIT {
        return (detail, false);
    }
    let mut end = FAILURE_DETAIL_LIMIT;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail.truncate(end);
    detail.push_str(" [truncated]");
    (detail, true)
}

fn trace_index(detail: &str) -> Option<usize> {
    for prefix in [
        "TimeAware logic trace differs at ",
        "ordered device read/write trace differs at ",
    ] {
        let Some(start) = detail.find(prefix).map(|index| index + prefix.len()) else {
            continue;
        };
        let rest = &detail[start..];
        let end = rest
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Ok(index) = rest[..end].parse() {
            return Some(index);
        }
    }
    None
}

impl Host {
    /// Corpus diagnostics only. A collected failure remains a mismatch, and its
    /// root entry hook is disabled until reset. Callee dispatch is unchanged.
    /// Callers must preserve/report `checks.failures` and exit unsuccessfully if
    /// `checks.mismatches != 0`; completing more frames never constitutes a pass.
    pub fn set_check_failure_policy(&mut self, policy: CheckFailurePolicy) -> Result<()> {
        if policy == CheckFailurePolicy::ContinueOriginal && self.mode != Mode::Check {
            return Err(HostError::Metadata(
                "ContinueOriginal is available only in check mode".into(),
            ));
        }
        self.check_failure_policy = policy;
        self.checks.failure_policy = policy;
        Ok(())
    }

    pub(super) fn check_live_hook(&mut self, hook: &Hook) -> Result<()> {
        if self.check_failure_policy == CheckFailurePolicy::FailFast {
            return self.check_hook(hook).map(|_| ());
        }
        let calls_before = self.checks.calls;
        let mismatches_before = self.checks.mismatches;
        if let Err(error) = self.check_hook(hook) {
            // Side-execution/metadata errors may happen before comparison and
            // before its counters. Count this failed attempt exactly once too.
            if self.checks.calls == calls_before {
                self.checks.calls += 1;
                self.checks.record_function(hook, true);
            }
            if self.checks.mismatches == mismatches_before {
                self.checks.record_mismatch(hook);
            }
            let entry_pc = hook.symbol.address(self.revision);
            self.disabled_check_roots.insert(entry_pc);
            let detail = error.to_string();
            let trace_index = trace_index(&detail);
            let (detail, detail_truncated) = bounded_detail(detail);
            let vdp_mismatch = self
                .checks
                .last_vdp_mismatch
                .as_ref()
                .filter(|report| report.frame == self.frame && report.function == hook.symbol.name)
                .cloned();
            self.checks.failures.push(CheckFailure {
                frame: self.frame,
                function: hook.symbol.name.to_owned(),
                entry_pc,
                master_clock: self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                frame_write_index: self
                    .emulator
                    .hybrid_bus()
                    .hybrid_writes
                    .as_ref()
                    .map_or(0, Vec::len),
                trace_index,
                detail,
                detail_truncated,
                vdp_mismatch,
            });
        }
        // No frame retry, CPU rewrite, or extra tick here. The caller falls
        // through to its one original instruction/tick in the current loop.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_text_and_trace_ordinals_are_bounded_without_losing_utf8() {
        let text = "long diagnostic é".repeat(400);
        let (detail, truncated) = bounded_detail(text);
        assert!(truncated);
        assert!(detail.len() <= FAILURE_DETAIL_LIMIT + 12);
        assert!(detail.ends_with(" [truncated]"));
        assert_eq!(
            trace_index("check mismatch: TimeAware logic trace differs at 42: value"),
            Some(42)
        );
        assert_eq!(
            trace_index("ordered device read/write trace differs at 7: value"),
            Some(7)
        );
        assert_eq!(
            trace_index("VDP state differs; first difference: fifo.word index=2"),
            None
        );
    }
}
