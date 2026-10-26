//! Additional measured domains for an independently successful strict check.
//! The actual live witness is never phase-repaired, normalized or reconstructed.
use super::{
    ledger::OutputCoverage,
    record::{Access, Direction, Event, Invocation},
};
use crate::{
    abi::{AtomicRun, OrderedWrite, canonical_ram_address},
    check_envelope::CheckForkRoute,
    check_projection::{LogicEvent, LogicTrace},
};
use genesis_core::{GenesisEmulator, HybridAccess};
use std::{collections::BTreeSet, ops::Range};
use tje_engine_core::ir::StateField;

struct OwnedEffects {
    device: Vec<Access>,
    ram: Vec<(u32, u8)>,
}

/// A fresh request for one admitted canonical body's actual analysis fork.
/// This is not evidence; only HookContext's observed execution can certify it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateRequest {
    owner: super::RootIdentity,
    fork: super::Attempt,
    // PC, SR, USP, SSP, exception depth. General inputs retain the strict ABI.
    entry: [u32; 5],
}
impl CandidateRequest {
    pub(super) fn new(
        admission: &super::RootAdmission,
        original: &Invocation,
    ) -> Result<Self, super::Unsupported> {
        super::ledger::CaseKey::root(admission, original)?;
        if admission.identity().kind != super::RootKind::StrictCanonical {
            return Err(super::Unsupported {
                kind: super::UnsupportedKind::CalleeCoverage,
                detail: "stack-alignment request requires an admitted canonical strict body".into(),
            });
        }
        let cpu = original.entry.boundary.cpu;
        Ok(Self {
            owner: admission.identity(),
            fork: admission.derived_identity().attempt,
            entry: [
                cpu.pc(),
                cpu.sr(),
                cpu.words[15],
                cpu.words[16],
                cpu.exception_depth,
            ],
        })
    }
    fn cpu_entry(cpu: &m68000_emu::M68000) -> [u32; 5] {
        [
            cpu.pc(),
            u32::from(cpu.status_register()),
            cpu.user_stack_pointer(),
            cpu.supervisor_stack_pointer(),
            cpu.hybrid_exception_depth(),
        ]
    }
    pub(crate) fn matches_hook(&self, hook: &crate::hooks::Hook) -> bool {
        self.owner.kind == super::RootKind::StrictCanonical
            && self.owner.function == hook.symbol.id
            && self.owner.body == hook.body_identity()
            && self.owner.class == hook.execution_class
            && self.owner.envelope == hook.proof_envelope
    }
    fn matches_root(&self, admission: &super::RootAdmission, original: &Invocation) -> bool {
        let cpu = original.entry.boundary.cpu;
        self.owner == admission.identity()
            && self.owner.invocation == original.entry.identity
            && self.entry
                == [
                    cpu.pc(),
                    cpu.sr(),
                    cpu.words[15],
                    cpu.words[16],
                    cpu.exception_depth,
                ]
    }
}

/// Owned by one requested Check fork. Odd attempts and opaque execution poison
/// it permanently, including odd-then-even restoration to the same endpoint.
#[derive(Debug, Clone)]
pub(crate) struct FrozenStackAlignment {
    request: CandidateRequest,
    eligible: bool,
}
impl FrozenStackAlignment {
    pub(crate) fn begin(request: CandidateRequest, cpu: &m68000_emu::M68000) -> Self {
        let eligible = request.entry == CandidateRequest::cpu_entry(cpu)
            && cpu.status_register() & 0x2000 != 0
            && cpu.supervisor_stack_pointer() & 1 == 0;
        Self { request, eligible }
    }
    pub(crate) fn observe(&mut self, cpu: &m68000_emu::M68000) {
        self.eligible &=
            cpu.status_register() & 0x2000 != 0 && cpu.supervisor_stack_pointer() & 1 == 0;
    }
    pub(crate) fn attempt(&mut self, value: u32) {
        self.eligible &= value & 1 == 0;
    }
    pub(crate) fn poison(&mut self) {
        self.eligible = false;
    }
    fn certifies(&self, request: &CandidateRequest) -> bool {
        self.eligible && self.request == *request
    }
}
pub(crate) struct CandidateObservation {
    words: [u32; StateField::COUNT],
    effects: Option<OwnedEffects>,
    request: CandidateRequest,
    frozen_stack_aligned: bool,
}
impl CandidateObservation {
    pub(super) fn matches_request(&self, request: &CandidateRequest) -> bool {
        self.request == *request
    }
    pub(super) fn frozen_stack_aligned(
        &self,
        admission: &super::RootAdmission,
        original: &Invocation,
    ) -> bool {
        self.request.matches_root(admission, original) && self.frozen_stack_aligned
    }
}
pub(crate) fn observe_candidate(
    candidate: &GenesisEmulator,
    route: CheckForkRoute,
    audit: &AtomicRun,
    trace: &LogicTrace,
    request: CandidateRequest,
) -> CandidateObservation {
    CandidateObservation {
        frozen_stack_aligned: audit
            .frozen_stack_alignment
            .as_ref()
            .is_some_and(|observed| observed.certifies(&request)),
        request,
        words: candidate.hybrid_cpu().hybrid_state_words(),
        effects: match route {
            CheckForkRoute::FrozenAtomic | CheckForkRoute::FrozenFunctional => {
                frozen_effects(candidate, audit)
            }
            _ => owned_effects(trace),
        },
    }
}
fn bus_access(event: HybridAccess) -> Option<Access> {
    match event {
        HybridAccess::Read {
            address,
            word,
            value,
            purpose,
        } => Some(Access {
            address,
            word,
            value,
            direction: Direction::Read(purpose),
        }),
        HybridAccess::Write {
            address,
            word,
            value,
        } => Some(Access {
            address,
            word,
            value,
            direction: Direction::Write,
        }),
        _ => None,
    }
}
fn frozen_effects(candidate: &GenesisEmulator, audit: &AtomicRun) -> Option<OwnedEffects> {
    // Frozen contracts run neither IRQs nor the Z80. Retain the real device
    // journal and the ordered committed byte-write audit, not an empty timed
    // LogicTrace or a synthesized comparison of final RAM values.
    let journal = candidate.hybrid_bus().hybrid_accesses.as_deref()?;
    let mut device = Vec::new();
    for &event in journal {
        let access = bus_access(event)?;
        if (0x400000..0xe00000).contains(&access.address) {
            device.push(access);
        }
    }
    let ram = audit
        .writes
        .iter()
        .filter_map(|write| match *write {
            OrderedWrite::Ram { address, value } => Some((canonical_ram_address(address), value)),
            _ => None,
        })
        .collect();
    Some(OwnedEffects { device, ram })
}
fn owned_effects(trace: &LogicTrace) -> Option<OwnedEffects> {
    let mut depth = 0_usize;
    let mut device = Vec::new();
    let mut ram = Vec::new();
    for event in &trace.events {
        match event {
            LogicEvent::ExceptionEntered { .. } => depth = depth.checked_add(1)?,
            LogicEvent::ExceptionReturned { .. } => depth = depth.checked_sub(1)?,
            LogicEvent::Access { access, .. } if depth == 0 => {
                let Some(access) = bus_access(*access) else {
                    continue;
                };
                if (0x400000..0xe00000).contains(&access.address) {
                    device.push(access);
                }
                if access.ram() && access.direction == Direction::Write {
                    ram.extend(
                        access
                            .bytes()
                            .map(|(a, byte)| (canonical_ram_address(a), byte)),
                    );
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(OwnedEffects { device, ram })
}

struct ActualEffects {
    device: Vec<Access>,
    ram: Vec<(u32, u8)>,
    private: Vec<Range<u32>>,
    public: BTreeSet<u32>,
    active: Vec<Range<u32>>,
}
impl ActualEffects {
    fn new() -> Self {
        Self {
            device: Vec::new(),
            ram: Vec::new(),
            private: Vec::new(),
            public: BTreeSet::new(),
            active: Vec::new(),
        }
    }
    fn matches_ram(&self, effects: &OwnedEffects) -> bool {
        // Only witnessed private-frame domains may differ. Incoming mutable
        // slots and address-taken caller locals lie outside the invoked frame.
        // A public write stays public even if a later allocation aliases it.
        effects
            .ram
            .iter()
            .copied()
            .filter(|(address, _)| {
                self.public.contains(address)
                    || !self.private.iter().any(|frame| frame.contains(address))
            })
            .eq(self.ram.iter().copied())
    }
    fn invocation(&mut self, original: &Invocation) {
        let high = canonical_ram_address(original.entry.boundary.cpu.sp());
        let mut lowest = high;
        self.active.push(high..high);
        for event in &original.trace.events {
            match event {
                Event::Instruction(index) => {
                    let instruction = &original.trace.instructions[*index];
                    let before = canonical_ram_address(instruction.before.cpu.sp()).min(high);
                    let after = canonical_ram_address(instruction.after_sp).min(high);
                    let low = before.min(after);
                    lowest = lowest.min(low);
                    self.active.last_mut().expect("actual own frame").start = low;
                    for &access in &original.trace.issues[instruction.issues.clone()] {
                        if (0x400000..0xe00000).contains(&access.address) {
                            self.device.push(access);
                        }
                        if access.ram() && access.direction == Direction::Write {
                            for (address, byte) in access.bytes() {
                                let address = canonical_ram_address(address);
                                if !self.active.iter().any(|frame| frame.contains(&address)) {
                                    self.ram.push((address, byte));
                                    self.public.insert(address);
                                }
                            }
                        }
                    }
                    self.active.last_mut().expect("actual own frame").start = after;
                }
                Event::Callee(child) => {
                    self.invocation(child);
                    self.active.last_mut().expect("actual caller frame").start =
                        canonical_ram_address(child.exit.cpu.sp()).min(high);
                }
                // Original IRQ work is retained in its immutable environment
                // node. It is not evidence that readable main code wrote it.
                Event::Interrupt(_) | Event::Idle(_) => {}
            }
        }
        self.active.pop().expect("actual frame closes");
        if lowest < high {
            self.private.push(lowest..high);
        }
    }
}

pub(super) fn coverage(original: &Invocation, candidate: &CandidateObservation) -> OutputCoverage {
    let words = &candidate.words;
    let mut covered = OutputCoverage::default();
    for (index, &word) in words.iter().enumerate() {
        let field = StateField::from_index(index).expect("canonical measured state field");
        let mask = match field {
            StateField::Data(_) | StateField::Address(_) | StateField::Usp | StateField::Ssp => {
                u32::MAX
            }
            StateField::Sr => 0xffff,
            _ => 0,
        };
        covered
            .state
            .insert(field, !(word ^ original.exit.cpu.words[index]) & mask);
    }
    if let Some(effects) = &candidate.effects {
        let mut actual = ActualEffects::new();
        actual.invocation(original);
        covered.nonframe_ram = actual.matches_ram(effects);
        covered.ordered_device_effects = effects.device == actual.device;
    }
    covered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Host, abi::StackAuditState, sinks::Inputs};
    const OUT: u32 = 0xff9000;
    const EXTRA: u32 = OUT + 2;

    fn candidate(writes: &[(u32, u8)]) -> (Host, AtomicRun) {
        let mut host = crate::fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
        for _ in 0..8 {
            if host.cpu().stack_pointer() == crate::catalogue::STACK_TOP {
                break;
            }
            host.tick_live_original(&mut Inputs(Default::default()))
                .unwrap();
        }
        let sp = host.cpu().stack_pointer();
        assert_eq!(sp, crate::catalogue::STACK_TOP);
        // Equal final public RAM cannot stand in for actual write evidence.
        {
            let mut context = host.context();
            context.write_u8(OUT, 7);
            context.write_u8(OUT + 1, 9);
            context.write_u8(EXTRA, 1);
        }
        host.emulator.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
        let audit = {
            let mut context = host.context();
            context.enable_stack_audit(StackAuditState::new(sp, false), AtomicRun::default());
            for &(address, value) in writes {
                context.write_u8(address, value);
            }
            assert!(
                context.logic_trace.borrow().events.is_empty(),
                "frozen writes must not depend on a fabricated timed LogicTrace"
            );
            context.audit.into_inner()
        };
        (host, audit)
    }
    fn actual() -> ActualEffects {
        let mut actual = ActualEffects::new();
        actual.ram.extend([(OUT, 7), (OUT + 1, 9)]);
        actual.public.extend([OUT, OUT + 1]);
        actual.private.push(0xff7fc0..0xff8000);
        actual
    }
    #[test]
    fn frozen_outparams_require_committed_ordered_audit_not_equal_final_ram() {
        for (writes, expected) in [
            (&[(OUT, 7), (OUT + 1, 9)][..], true),
            (&[][..], false),
            (&[(OUT, 7)][..], false),
            (&[(OUT, 7), (OUT + 1, 9), (EXTRA, 1)][..], false),
            (&[(OUT + 1, 9), (OUT, 7)][..], false),
            (&[(OUT, 7), (OUT + 1, 9), (OUT + 1, 9)][..], false),
            (&[(0xff7fd0, 0x55), (OUT, 7), (OUT + 1, 9)][..], true),
        ] {
            let (mut host, audit) = candidate(writes);
            assert_eq!(host.context().read_u16(OUT), 0x0709);
            assert_eq!(host.context().read_u8(EXTRA), 1);
            let effects = frozen_effects(&host.emulator, &audit).unwrap();
            assert_eq!(
                actual().matches_ram(&effects),
                expected,
                "actual committed writes {writes:?}"
            );
        }
    }
    #[test]
    fn absent_or_foreign_frozen_journal_never_becomes_known_empty_coverage() {
        let (mut host, audit) = candidate(&[(OUT, 7), (OUT + 1, 9)]);
        host.emulator.hybrid_bus_mut().hybrid_accesses = None;
        assert!(frozen_effects(&host.emulator, &audit).is_none());
        host.emulator.hybrid_bus_mut().hybrid_accesses = Some(vec![HybridAccess::Z80Write {
            address: OUT,
            word: false,
            value: 7,
        }]);
        assert!(frozen_effects(&host.emulator, &audit).is_none());
    }
    #[test]
    fn public_outparam_ownership_survives_a_later_private_alias() {
        let (host, audit) = candidate(&[]);
        let effects = frozen_effects(&host.emulator, &audit).unwrap();
        let mut actual = actual();
        actual.private.push(OUT..OUT + 2);
        assert!(!actual.matches_ram(&effects));
    }
    #[test]
    fn frozen_device_evidence_retains_raw_value_purpose_width_and_order() {
        let (mut host, _) = candidate(&[]);
        let audit;
        let value;
        {
            let sp = host.cpu().stack_pointer();
            let mut context = host.context();
            context.enable_stack_audit(StackAuditState::new(sp, false), AtomicRun::default());
            value = tje_game::Platform::read_device_u8(&mut context, 0xa10001);
            tje_game::Platform::write_device_u8(&mut context, 0xc00011, 0x90);
            audit = context.audit.into_inner();
        }
        let mut effects = frozen_effects(&host.emulator, &audit).unwrap();
        let expected = vec![
            Access {
                address: 0xa10001,
                word: false,
                value: u16::from(value),
                direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
            },
            Access {
                address: 0xc00011,
                word: false,
                value: 0x90,
                direction: Direction::Write,
            },
        ];
        assert_eq!(effects.device, expected);
        effects.device[0].direction = Direction::Read(m68000_emu::ReadPurpose::Discarded);
        assert_ne!(effects.device, expected);
        effects.device[0].direction = Direction::Read(m68000_emu::ReadPurpose::Semantic);
        effects.device.reverse();
        assert_ne!(effects.device, expected);
    }

    #[cfg(feature = "reference-proof-hooks")]
    #[test]
    fn canonical_frozen_alignment_is_bound_to_the_actual_body_fork_and_entry() {
        for revision in [tje_rom::Revision::Rev02, tje_rom::Revision::Rev00] {
            let (mut host, mut completed) =
                crate::replayed_environment::tests::actual_strict_child(revision);
            let request =
                CandidateRequest::new(&completed.root.admission, &completed.original).unwrap();
            let expected = request.clone();
            let other_fork =
                CandidateRequest::new(&completed.root.admission, &completed.original).unwrap();
            let (_other_host, other) =
                crate::replayed_environment::tests::actual_strict_child(revision);
            host.with_strict_child_analysis(&mut completed.root, |analysis, hook| {
                // This positive uses the real unchanged canonical body and its
                // complete ordinary strict checker, not the fault seam below.
                let (_, candidate) = analysis.check_hook_observed(hook, Some(request))?;
                let candidate = candidate.expect("requested actual candidate");
                assert!(candidate.frozen_stack_aligned);
                assert!(candidate.matches_request(&expected));
                assert!(!candidate.matches_request(&other_fork));
                assert!(!candidate.frozen_stack_aligned(&other.root.admission, &other.original));
                Ok(candidate)
            })
            .unwrap();
        }
    }

    #[cfg(feature = "reference-proof-hooks")]
    #[test]
    fn frozen_alignment_observer_seam_latches_all_stack_and_opaque_path_attempts() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use tje_game::{Context, Platform};
        for revision in [tje_rom::Revision::Rev02, tje_rom::Revision::Rev00] {
            for variant in 0..=10 {
                let (mut host, mut completed) =
                    crate::replayed_environment::tests::actual_strict_child(revision);
                let request =
                    CandidateRequest::new(&completed.root.admission, &completed.original).unwrap();
                let calls = host.checks.calls;
                host.with_strict_child_analysis(&mut completed.root, |analysis, hook| {
                    let entry_sp = analysis.emulator.hybrid_cpu().stack_pointer();
                    if variant == 8 {
                        crate::abi::set_sp(analysis.emulator.hybrid_parts().0, entry_sp | 1);
                    }
                    let audit = {
                        let mut context = crate::abi::HookContext::new(
                            &mut analysis.emulator,
                            revision,
                            analysis.registry.dispatcher(),
                            crate::Mode::Check,
                        );
                        context.enable_stack_audit(
                            StackAuditState::new(entry_sp, false),
                            AtomicRun::default(),
                        );
                        context.observe_frozen_stack_alignment(request.clone());
                        match variant {
                            0 => {}
                            1 => {
                                context.set_stack_pointer(entry_sp - 4).unwrap();
                                context.set_stack_pointer(entry_sp).unwrap();
                            }
                            2 | 8 => {
                                if variant == 2 {
                                    context.set_stack_pointer(entry_sp | 1).unwrap();
                                }
                                context.set_stack_pointer(entry_sp).unwrap();
                            }
                            3 => {
                                let banks = context.stack_banks().unwrap();
                                context
                                    .set_stack_banks(tje_game::exits::StackBanks {
                                        supervisor: banks.supervisor | 1,
                                        ..banks
                                    })
                                    .unwrap();
                                context.set_stack_banks(banks).unwrap();
                            }
                            4 => {
                                let sr = context.status_register();
                                context.set_status_register(sr & !0x2000);
                                context.set_status_register(sr);
                            }
                            5 => {
                                assert!(
                                    catch_unwind(AssertUnwindSafe(|| {
                                        context.set_address_register(7, entry_sp | 1);
                                    }))
                                    .is_err()
                                );
                                context.set_stack_pointer(entry_sp).unwrap();
                            }
                            6 => {
                                let _opaque_lend = context.cpu();
                            }
                            7 => {
                                context
                                    .call_original(
                                        tje_game::ids::ABS.metadata().address(revision),
                                        &[9],
                                    )
                                    .unwrap();
                            }
                            9 => {
                                assert!(
                                    context
                                        .wait_until(tje_game::execution::WaitRequest::interrupt())
                                        .is_err()
                                );
                            }
                            10 => {
                                context.transfer_to(hook.symbol.address(revision)).unwrap();
                            }
                            _ => unreachable!(),
                        }
                        assert_eq!(context.stack_pointer().unwrap(), entry_sp);
                        context.audit.into_inner()
                    };
                    // Fault instrumentation surrounds an authenticated real
                    // child/fork, but is NOT a canonical-body or ledger proof.
                    let candidate = observe_candidate(
                        &analysis.emulator,
                        CheckForkRoute::FrozenAtomic,
                        &audit,
                        &LogicTrace::default(),
                        request,
                    );
                    assert_eq!(
                        candidate.frozen_stack_aligned,
                        variant <= 1,
                        "{revision:?} variant {variant}"
                    );
                    Ok(candidate)
                })
                .unwrap();
                assert_eq!(
                    host.checks.calls, calls,
                    "observer seam records no strict/campaign case credit"
                );
            }
        }
    }

    #[cfg(feature = "reference-proof-hooks")]
    #[test]
    fn frozen_alignment_capture_is_requested_check_only_and_rejects_foreign_fork_evidence() {
        use tje_game::Context;
        for revision in [tje_rom::Revision::Rev02, tje_rom::Revision::Rev00] {
            for mode in [crate::Mode::Check, crate::Mode::Replace] {
                let (mut host, mut completed) =
                    crate::replayed_environment::tests::actual_strict_child(revision);
                let request =
                    CandidateRequest::new(&completed.root.admission, &completed.original).unwrap();
                let foreign =
                    CandidateRequest::new(&completed.root.admission, &completed.original).unwrap();
                host.with_strict_child_analysis(&mut completed.root, |analysis, _hook| {
                    let audit = {
                        let sp = analysis.emulator.hybrid_cpu().stack_pointer();
                        let mut context = crate::abi::HookContext::new(
                            &mut analysis.emulator,
                            revision,
                            analysis.registry.dispatcher(),
                            mode,
                        );
                        context.enable_stack_audit(
                            StackAuditState::new(sp, false),
                            AtomicRun::default(),
                        );
                        context.set_stack_pointer(sp).unwrap();
                        assert!(
                            context.audit.get_mut().frozen_stack_alignment.is_none(),
                            "no request, no observation"
                        );
                        context.observe_frozen_stack_alignment(request);
                        assert_eq!(
                            context.audit.get_mut().frozen_stack_alignment.is_some(),
                            mode == crate::Mode::Check
                        );
                        context.audit.into_inner()
                    };
                    let candidate = observe_candidate(
                        &analysis.emulator,
                        CheckForkRoute::FrozenAtomic,
                        &audit,
                        &LogicTrace::default(),
                        foreign,
                    );
                    assert!(
                        !candidate.frozen_stack_aligned,
                        "same body/entry cannot borrow another fork's witness"
                    );
                    Ok(candidate)
                })
                .unwrap();
            }
        }
    }
}
