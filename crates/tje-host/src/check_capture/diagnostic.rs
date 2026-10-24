//! Failure evidence distinguishes an unexecuted counterpart from an observed
//! zero-IRQ execution. These records never grant projection approval.
use super::{CheckCaptureEvidence, Side};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", content = "evidence", rename_all = "snake_case")]
pub enum CapturedSide {
    NotExecuted,
    Completed(Box<CheckCaptureEvidence>),
    Failed(Box<CheckCaptureEvidence>),
}
impl CapturedSide {
    fn from_evidence(evidence: Box<CheckCaptureEvidence>) -> Self {
        if evidence.execution_complete {
            Self::Completed(evidence)
        } else {
            Self::Failed(evidence)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckCaptureDiagnostic {
    pub function: String,
    pub comparison: Option<&'static str>,
    pub comparison_error: Option<String>,
    pub original: CapturedSide,
    pub translated: CapturedSide,
    /// Baselines are never overwritten by speculative probe evidence. Keep each
    /// failed probe's own side/purpose/origin and error, including both sides.
    pub failed_probes: Vec<CheckCaptureEvidence>,
    pub grouping_issues: Vec<String>,
    pub admission: &'static str,
}
impl CheckCaptureDiagnostic {
    pub(crate) fn new(function: &str) -> Self {
        Self {
            function: function.to_owned(),
            comparison: None,
            comparison_error: None,
            original: CapturedSide::NotExecuted,
            translated: CapturedSide::NotExecuted,
            failed_probes: Vec::new(),
            grouping_issues: Vec::new(),
            admission: "capture_only_no_projection_approval",
        }
    }

    /// Same physical lanes, observed on the other completed fork. This is not
    /// relative mapping, ownership, pairing, or a predicted wrapper.
    pub(crate) fn attach_counterpart_frames(
        &mut self,
        original: &genesis_core::GenesisEmulator,
        translated: &genesis_core::GenesisEmulator,
    ) {
        fn checkpoint(side: &CapturedSide) -> Option<super::CpuCheckpoint> {
            match side {
                CapturedSide::Completed(evidence) => evidence.final_observer_checkpoint,
                _ => None,
            }
        }
        fn attach(
            side: &mut CapturedSide,
            other: &genesis_core::GenesisEmulator,
            checkpoint: super::CpuCheckpoint,
        ) {
            let CapturedSide::Completed(evidence) = side else {
                return;
            };
            for exception in &mut evidence.exceptions {
                let Some(image) = &exception.after_frame else {
                    continue;
                };
                let Some(end) = u32::try_from(image.bytes.len())
                    .ok()
                    .and_then(|len| image.start.checked_add(len))
                else {
                    exception
                        .issues
                        .push("counterpart physical frame range overflows".into());
                    continue;
                };
                if image.start < crate::catalogue::STACK_BOTTOM || end > crate::catalogue::STACK_TOP
                {
                    exception
                        .issues
                        .push("counterpart physical lanes leave the known stack".into());
                    continue;
                }
                let raw = other.hybrid_bus().memory.hybrid_raw_main_ram();
                let bytes = (image.start..end)
                    .map(|address| super::physical_ram_byte(raw, address))
                    .collect();
                exception.counterpart_final_frame = Some(super::MemorySnapshot {
                    start: image.start,
                    bytes,
                    checkpoint,
                });
            }
        }
        if let (Some(original_checkpoint), Some(translated_checkpoint)) =
            (checkpoint(&self.original), checkpoint(&self.translated))
        {
            attach(&mut self.original, translated, translated_checkpoint);
            attach(&mut self.translated, original, original_checkpoint);
        }
    }

    pub(crate) fn record(&mut self, evidence: Box<CheckCaptureEvidence>, baseline: bool) {
        if evidence.binding.function != self.function {
            self.grouping_issues.push(format!(
                "attempt for {} does not belong to root {}",
                evidence.binding.function, self.function
            ));
            self.failed_probes.push(*evidence);
            return;
        }
        if !baseline {
            if !evidence.execution_complete {
                self.failed_probes.push(*evidence);
            }
            return;
        }
        let target = match evidence.binding.side {
            Side::Original => &mut self.original,
            Side::Rust => &mut self.translated,
        };
        if !matches!(target, CapturedSide::NotExecuted) {
            self.grouping_issues
                .push("duplicate baseline attempt for the same actual side".into());
            self.failed_probes.push(*evidence);
            return;
        }
        *target = CapturedSide::from_evidence(evidence);
    }
}
