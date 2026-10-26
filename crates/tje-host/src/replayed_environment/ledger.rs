//! Conditional own-function matches are not proofs until every exact sealed
//! callee case has completed at an accepted tier. Counts cannot discharge cases.
use super::{
    Profile, RootAdmission, Trajectory, Unsupported, UnsupportedKind,
    record::{Invocation, InvocationId},
};
use crate::future_frame_watch::{DependencyOutcome, ProofDependency};
use crate::hooks::{BodyIdentity, Hook, ReplayBinding};
use std::collections::{BTreeMap, BTreeSet};
use tje_game::FunctionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CaseKey {
    function: FunctionId,
    body: BodyIdentity,
    /// A handle to the same sealed actual invocation, never a serialized
    /// counter or equality of independent reconstructed snapshots.
    observation: InvocationId,
    contract: [u8; 6],
}
impl CaseKey {
    fn contract(hook: &Hook, profile: Profile) -> Result<[u8; 6], Unsupported> {
        let selection = hook.replay_selection().ok_or_else(|| Unsupported {
            kind: UnsupportedKind::CalleeCoverage,
            detail: "coverage root has no static readable payload".into(),
        })?;
        let form = match selection.entry() {
            tje_game::TranslatedEntry::Values(_) => 0,
            tje_game::TranslatedEntry::Framed(_) => 1,
        };
        Ok([
            hook.execution_class as u8,
            hook.proof_envelope as u8,
            hook.check_route() as u8,
            profile as u8,
            form,
            hook.body_provenance() as u8,
        ])
    }
    pub fn root(admission: &RootAdmission, invocation: &Invocation) -> Result<Self, Unsupported> {
        let identity = admission.identity();
        if identity.invocation != invocation.entry.identity
            || invocation.entry.function != Some(identity.function)
        {
            return Err(Unsupported {
                kind: UnsupportedKind::CalleeCoverage,
                detail: "completed witness is not this admitted root's original invocation".into(),
            });
        }
        Ok(Self {
            function: identity.function,
            body: identity.body,
            observation: identity.invocation,
            contract: Self::contract(admission.hook(), identity.profile)?,
        })
    }
    /// Names an obligation only. This does not record a visit or proof.
    pub fn callee(
        binding: ReplayBinding,
        hook: &Hook,
        invocation: &Invocation,
    ) -> Result<Self, Unsupported> {
        if binding.id() != hook.symbol.id
            || binding.body() != hook.body_identity()
            || invocation.entry.function != Some(binding.id())
        {
            return Err(Unsupported { kind: UnsupportedKind::CalleeCoverage,
                detail: "effective callee route, registered root and actual invocation do not share their sealed payload/version".into() });
        }
        Ok(Self {
            function: binding.id(),
            body: binding.body(),
            observation: invocation.entry.identity,
            contract: Self::contract(hook, Profile::OrdinaryRts)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tier {
    Strict,
    Replayed,
}
impl Tier {
    fn satisfies(self, required: Self) -> bool {
        self == Self::Strict || required == Self::Replayed
    }
}

/// The actual checked output domain, not the ABI's *permission* to clobber it.
/// In particular ordinary C success does not authenticate scratch/CCR bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OutputCoverage {
    pub state: tje_engine_core::dependencies::StateMask,
    pub nonframe_ram: bool,
    pub ordered_device_effects: bool,
}
impl OutputCoverage {
    pub(super) fn covers(self, required: Self) -> bool {
        (required.state - self.state).is_empty()
            && (!required.nonframe_ram || self.nonframe_ram)
            && (!required.ordered_device_effects || self.ordered_device_effects)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Requirement {
    Checked {
        case: CaseKey,
        tier: Tier,
        outputs: OutputCoverage,
    },
    Uncovered {
        function: FunctionId,
        detail: String,
    },
}

#[derive(Debug, Clone)]
enum State {
    Waiting,
    Complete(Tier),
    Failed(Unsupported),
}
struct Node {
    own_tier: Option<Tier>,
    required: Vec<(CaseKey, Tier, OutputCoverage)>,
    outputs: OutputCoverage,
    uncovered: Vec<(FunctionId, String)>,
    provenance: Vec<ProofDependency>,
    state: State,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            own_tier: None,
            required: Vec::new(),
            outputs: OutputCoverage::default(),
            uncovered: Vec::new(),
            provenance: Vec::new(),
            state: State::Waiting,
        }
    }
}

pub(super) struct Ledger {
    trajectory: Trajectory,
    nodes: BTreeMap<CaseKey, Node>,
    parents: BTreeMap<CaseKey, BTreeSet<CaseKey>>,
    unresolved: u64,
}
impl Ledger {
    pub fn new(trajectory: Trajectory) -> Self {
        Self {
            trajectory,
            nodes: BTreeMap::new(),
            parents: BTreeMap::new(),
            unresolved: 0,
        }
    }
    fn node(&mut self, key: CaseKey) -> &mut Node {
        match self.nodes.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                self.unresolved += 1;
                entry.insert(Node::default())
            }
        }
    }
    fn check_key(&self, key: CaseKey) -> Result<(), Unsupported> {
        if key.observation.trajectory() == self.trajectory {
            Ok(())
        } else {
            Err(Unsupported {
                kind: UnsupportedKind::CalleeCoverage,
                detail: "proof belongs to another original trajectory/epoch".into(),
            })
        }
    }

    /// Called only after the independent root driver has passed its own exact
    /// access/frame/ABI/probe obligations. Substituting a callee never calls it.
    pub fn own_match(
        &mut self,
        key: CaseKey,
        tier: Tier,
        outputs: OutputCoverage,
        requirements: Vec<Requirement>,
        provenance: Vec<ProofDependency>,
    ) -> Result<(), Unsupported> {
        self.check_key(key)?;
        let mut checked = Vec::new();
        let mut uncovered = Vec::new();
        for requirement in requirements {
            match requirement {
                Requirement::Checked {
                    case,
                    tier,
                    outputs,
                } => {
                    self.check_key(case)?;
                    checked.push((case, tier, outputs));
                }
                Requirement::Uncovered { function, detail } => uncovered.push((function, detail)),
            }
        }
        // Re-observation cannot replace a failed/unsupported case or weaken its
        // original dependency set. Such attempts remain non-proofs.
        if let Some(node) = self.nodes.get(&key) {
            if let State::Failed(reason) = &node.state {
                return Err(reason.clone());
            }
            if node.own_tier.is_some() {
                if !node.provenance.is_empty() || !provenance.is_empty() {
                    return Err(Unsupported { kind: UnsupportedKind::CalleeCoverage,
                        detail: "a sealed scoped provenance journal cannot be replaced by a second observation".into() });
                }
                if node.own_tier != Some(tier)
                    || node.outputs != outputs
                    || node.required != checked
                    || node.uncovered != uncovered
                {
                    return Err(Unsupported {
                        kind: UnsupportedKind::CalleeCoverage,
                        detail: "same sealed case was supplied with different callee obligations"
                            .into(),
                    });
                }
                return Ok(());
            }
        }
        for (child, _, _) in &checked {
            self.node(*child);
            self.parents.entry(*child).or_default().insert(key);
        }
        let node = self.node(key);
        node.own_tier = Some(tier);
        node.outputs = outputs;
        node.required = checked;
        node.uncovered = uncovered;
        node.provenance = provenance;
        self.resolve_from(key);
        Ok(())
    }

    pub fn fail(&mut self, key: CaseKey, reason: Unsupported) -> Result<(), Unsupported> {
        self.check_key(key)?;
        let mut pending = vec![key];
        while let Some(case) = pending.pop() {
            let node = self.node(case);
            if matches!(node.state, State::Failed(_)) {
                continue;
            }
            let was_complete = matches!(node.state, State::Complete(_));
            node.state = State::Failed(reason.clone());
            if was_complete {
                self.unresolved += 1;
            }
            if let Some(parents) = self.parents.get(&case) {
                pending.extend(parents.iter().copied());
            }
        }
        Ok(())
    }

    fn resolve_from(&mut self, first: CaseKey) {
        let mut pending = vec![first];
        while let Some(case) = pending.pop() {
            let Some(node) = self.nodes.get(&case) else {
                continue;
            };
            if !matches!(node.state, State::Waiting) || !node.uncovered.is_empty() {
                continue;
            }
            if !node
                .provenance
                .iter()
                .all(|dependency| dependency.outcome() == DependencyOutcome::Discharged)
            {
                continue;
            }
            let Some(tier) = node.own_tier else {
                continue;
            };
            let complete = node.required.iter().all(|(child, required, outputs)| {
                self.nodes.get(child).is_some_and(|node| matches!(node.state,
                    State::Complete(actual) if actual.satisfies(*required) && node.outputs.covers(*outputs)))
            });
            if !complete {
                continue;
            }
            self.nodes
                .get_mut(&case)
                .expect("observed ledger case")
                .state = State::Complete(tier);
            self.unresolved -= 1;
            if let Some(parents) = self.parents.get(&case) {
                pending.extend(parents.iter().copied());
            }
        }
    }

    pub fn complete(&self, key: CaseKey) -> bool {
        self.nodes
            .get(&key)
            .is_some_and(|node| matches!(node.state, State::Complete(_)))
    }
    pub fn satisfies(&self, key: CaseKey, required: Tier, outputs: OutputCoverage) -> bool {
        self.nodes.get(&key).is_some_and(|node| {
            matches!(node.state,
            State::Complete(actual) if actual.satisfies(required) && node.outputs.covers(outputs))
        })
    }
    pub fn failed(&self, key: CaseKey) -> Option<&Unsupported> {
        self.nodes.get(&key).and_then(|node| match &node.state {
            State::Failed(reason) => Some(reason),
            _ => None,
        })
    }
    pub fn unresolved_count(&self) -> u64 {
        self.unresolved
    }
    pub fn refresh_provenance(&mut self) -> Result<(), Unsupported> {
        let cases: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, node)| !node.provenance.is_empty())
            .map(|(&case, _)| case)
            .collect();
        for case in cases {
            if self
                .nodes
                .get(&case)
                .expect("scoped provenance case")
                .provenance
                .iter()
                .any(|dependency| dependency.outcome() == DependencyOutcome::Invalidated)
            {
                self.fail(case, Unsupported { kind: UnsupportedKind::CalleeCoverage,
                    detail: "the exact retained future-frame dependency was invalidated; public reports cannot replace its graph".into() })?;
            } else {
                self.resolve_from(case);
            }
        }
        Ok(())
    }
    pub fn finish(&self) -> Result<(), Unsupported> {
        if let Some(reason) = self.nodes.values().find_map(|node| match &node.state {
            State::Failed(reason) => Some(reason.clone()),
            _ => None,
        }) {
            return Err(reason);
        }
        let unresolved = self.unresolved_count();
        if unresolved != 0 {
            return Err(Unsupported {
                kind: UnsupportedKind::CalleeCoverage,
                detail: format!(
                    "{unresolved} exact same-state callee/root obligations remain unresolved; isolated fixtures, unhooked/disabled roots and mismatched body/input/environment cases are not approval"
                ),
            });
        }
        Ok(())
    }
}
