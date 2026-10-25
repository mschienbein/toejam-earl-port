//! Explicit root selection for session proofs. Pending bodies never gain live eligibility.
use anyhow::{Result, bail, ensure};
use std::sync::{LazyLock, OnceLock};
use tje_game::{FramedTranslation, FunctionId, Translation};
use tje_host::{Mode, hooks::HookRegistry};
use tje_mem::Revision;

#[derive(Debug, Default)]
pub struct Selection {
    functions: Vec<FunctionId>,
    pending: bool,
}

#[derive(Default)]
struct Bodies {
    values: Vec<Translation>,
    framed: Vec<FramedTranslation>,
}

// These are the owners' public canonical pending tables, not a second body registry.
// Missing canonical bodies remain an explicit error, never original-only proof approval.
static PENDING_BODIES: LazyLock<Bodies> = LazyLock::new(|| Bodies {
    values: tje_game::pending_translations().copied().collect(),
    framed: tje_game::pending_framed_translations().copied().collect(),
});

// The hook API borrows static bodies. One CLI invocation has one immutable root
// selection; initialize it once without leaking a different allocation per session.
static SELECTED_PENDING: OnceLock<Bodies> = OnceLock::new();

pub(crate) fn pending_bodies() -> (&'static [Translation], &'static [FramedTranslation]) {
    (&PENDING_BODIES.values, &PENDING_BODIES.framed)
}

impl Selection {
    pub fn parse(names: &[String], pending: bool) -> Result<Self> {
        ensure!(
            !pending || !names.is_empty(),
            "--pending requires an explicit --function selection"
        );
        let mut functions = Vec::with_capacity(names.len());
        for name in names {
            let function = tje_game::functions()
                .find(|function| function.inventory_id == name || function.name == name)
                .ok_or_else(|| anyhow::anyhow!("unknown function inventory id/name: {name}"))?;
            ensure!(
                !functions.contains(&function.id),
                "duplicate function selection: {}",
                function.inventory_id
            );
            functions.push(function.id);
        }
        Ok(Self { functions, pending })
    }

    pub(crate) fn function_ids(&self) -> &[FunctionId] {
        &self.functions
    }

    pub fn registry(&self, mode: Mode, revision: Revision) -> Result<HookRegistry> {
        if mode == Mode::Reference {
            ensure!(
                self.functions.is_empty() && !self.pending,
                "function selection requires --mode check or replace"
            );
            return Ok(HookRegistry::empty(revision));
        }
        if !self.pending {
            return Ok(HookRegistry::filtered(revision, &[], &self.functions)?);
        }
        ensure!(mode == Mode::Check, "--pending requires --mode check");
        for &id in &self.functions {
            let count = PENDING_BODIES
                .values
                .iter()
                .filter(|entry| entry.id == id)
                .count()
                + PENDING_BODIES
                    .framed
                    .iter()
                    .filter(|entry| entry.id == id)
                    .count();
            ensure!(
                count == 1,
                "{} requires exactly one canonical pending body, found {count}",
                id.metadata().inventory_id
            );
        }
        let bodies = SELECTED_PENDING.get_or_init(|| Bodies {
            values: PENDING_BODIES
                .values
                .iter()
                .filter(|entry| self.functions.contains(&entry.id))
                .copied()
                .collect(),
            framed: PENDING_BODIES
                .framed
                .iter()
                .filter(|entry| self.functions.contains(&entry.id))
                .copied()
                .collect(),
        });
        let same_selection = bodies.values.len() + bodies.framed.len() == self.functions.len()
            && bodies
                .values
                .iter()
                .map(|entry| entry.id)
                .chain(bodies.framed.iter().map(|entry| entry.id))
                .all(|id| self.functions.contains(&id));
        if !same_selection {
            bail!("a process cannot change its immutable pending body selection");
        }
        Ok(HookRegistry::proof_filtered_with_frames(
            revision,
            &bodies.values,
            &bodies.framed,
            &self.functions,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_ids_and_names_resolve_to_the_same_function() {
        let by_id = Selection::parse(&["func_00059e".into()], true).unwrap();
        let by_name = Selection::parse(&["SelectViewportSpriteChain".into()], true).unwrap();
        assert_eq!(by_id.functions, by_name.functions);
        assert!(Selection::parse(&["not-a-function".into()], false).is_err());
        assert!(Selection::parse(&[], true).is_err());
        assert!(
            Selection::parse(
                &["func_00059e".into(), "SelectViewportSpriteChain".into()],
                true
            )
            .is_err()
        );
    }

    #[test]
    fn pending_selection_cannot_enable_replacement_or_filtered_reference() {
        let selected = Selection::parse(&["func_00059e".into()], true).unwrap();
        assert!(selected.registry(Mode::Replace, Revision::Rev02).is_err());
        assert!(selected.registry(Mode::Reference, Revision::Rev02).is_err());
    }

    #[test]
    fn selected_pending_value_and_frame_roots_register_in_both_revisions() {
        let (values, framed) = pending_bodies();
        let names = [
            values.first().expect("canonical pending value body").id,
            framed.first().expect("canonical pending framed body").id,
        ]
        .map(|id| id.metadata().inventory_id.to_owned());
        let selection = Selection::parse(&names, true).unwrap();
        for revision in [Revision::Rev00, Revision::Rev02] {
            let registry = selection.registry(Mode::Check, revision).unwrap();
            assert_eq!(registry.len(), names.len());
            for name in &names {
                assert!(registry.iter().any(|hook| hook.symbol.inventory_id == name));
            }
        }
    }
}
