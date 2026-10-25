//! Check-only replay worker. Host-owned packets remain output, never imported proof authority.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{fs, io::Write, path::Path, sync::Arc};
use tje_game::{FramedTranslation, FunctionId, Translation};
use tje_host::{CheckFailurePolicy, Host, Mode, hooks::HookRegistry};
use tje_mem::Revision;
use tje_session::Session;

use crate::{runner, selection};

#[derive(Debug)]
enum ConstructionError {
    Unsupported(String),
    Invalid(String),
}

trait RegistryFactory {
    type Registry;
    fn construct(
        &mut self,
        revision: Revision,
        pending: &'static [Translation],
        framed: &'static [FramedTranslation],
        functions: &[FunctionId],
    ) -> std::result::Result<Self::Registry, ConstructionError>;
}

struct NativeFactory;
impl RegistryFactory for NativeFactory {
    type Registry = HookRegistry;

    fn construct(
        &mut self,
        revision: Revision,
        pending: &'static [Translation],
        framed: &'static [FramedTranslation],
        functions: &[FunctionId],
    ) -> std::result::Result<Self::Registry, ConstructionError> {
        if functions.is_empty() {
            return Err(ConstructionError::Invalid(
                "an empty explicit replay filter would widen the requested scope".into(),
            ));
        }
        #[cfg(feature = "replayed-environment")]
        {
            HookRegistry::proof_replayed_environment(revision, pending, framed, functions).map_err(
                |error| match error {
                    error @ tje_host::HostError::ReplayUnsupported { .. } => {
                        ConstructionError::Unsupported(error.to_string())
                    }
                    error => ConstructionError::Invalid(error.to_string()),
                },
            )
        }
        #[cfg(not(feature = "replayed-environment"))]
        {
            let _ = (revision, pending, framed, functions);
            Err(ConstructionError::Unsupported(
                "binary lacks the replayed-environment feature; the real Host envelope is required"
                    .into(),
            ))
        }
    }
}

#[derive(Debug, Serialize)]
struct Exclusion {
    inventory_id: &'static str,
    name: &'static str,
    kind: &'static str,
    detail: String,
}

struct Plan<R> {
    registry: Option<R>,
    supported: Vec<FunctionId>,
    exclusions: Vec<Exclusion>,
}

fn prepare<F: RegistryFactory>(
    factory: &mut F,
    revision: Revision,
    pending: &'static [Translation],
    framed: &'static [FramedTranslation],
    requested: &[FunctionId],
) -> Result<Plan<F::Registry>> {
    ensure!(
        !requested.is_empty(),
        "replay proof requires explicit requested roots"
    );
    let mut supported = Vec::with_capacity(requested.len());
    let mut exclusions = Vec::new();
    for &function in requested {
        match factory.construct(revision, pending, framed, &[function]) {
            Ok(_) => supported.push(function),
            Err(ConstructionError::Unsupported(detail)) => exclusions.push(Exclusion {
                inventory_id: function.metadata().inventory_id,
                name: function.metadata().name,
                kind: "constructor_unsupported",
                detail,
            }),
            Err(ConstructionError::Invalid(detail)) => {
                anyhow::bail!(
                    "invalid replay registry for {}: {detail}",
                    function.metadata().inventory_id
                );
            }
        }
    }
    // Empty filters mean "all" to registry constructors. Never accidentally
    // construct that scope after every originally requested root was excluded.
    let registry = if supported.is_empty() {
        None
    } else {
        Some(
            factory
                .construct(revision, pending, framed, &supported)
                .map_err(|error| anyhow::anyhow!("combined replay registry rejected: {error:?}"))?,
        )
    };
    Ok(Plan {
        registry,
        supported,
        exclusions,
    })
}

fn metadata(function: FunctionId) -> Value {
    let entry = function.metadata();
    let (values, framed) = selection::pending_bodies();
    let form = if values.iter().any(|body| body.id == function) {
        Some("values")
    } else if framed.iter().any(|body| body.id == function) {
        Some("framed")
    } else {
        None
    };
    #[cfg(feature = "replayed-environment")]
    let replayed_form = if tje_game::replayed_translations().any(|body| body.id == function) {
        Some("values")
    } else if tje_game::replayed_framed_translations().any(|body| body.id == function) {
        Some("framed")
    } else {
        None
    };
    #[cfg(not(feature = "replayed-environment"))]
    let replayed_form: Option<&str> = None;
    let actual_form = form.or(replayed_form).or_else(|| {
        if tje_game::translations().any(|body| body.id == function) {
            Some("values")
        } else if tje_game::framed_translations().any(|body| body.id == function) {
            Some("framed")
        } else {
            None
        }
    });
    serde_json::json!({
        "inventory_id":entry.inventory_id, "name":entry.name,
        "subsystem":entry.subsystem, "pending_body_form":form,
        "replayed":replayed_form.is_some(), "body_form":actual_form,
    })
}

pub fn catalogue() -> Value {
    let production: std::collections::BTreeSet<_> = tje_game::translations()
        .map(|body| body.id)
        .chain(tje_game::framed_translations().map(|body| body.id))
        .collect();
    let rows: Vec<_> = tje_game::functions()
        .map(|function| {
            let mut row = metadata(function.id);
            row["production"] = production.contains(&function.id).into();
            row
        })
        .collect();
    serde_json::json!({
        "schema_version":1, "protocol":"tje-replayed-worker/1",
        "replayed_environment_available":cfg!(feature="replayed-environment"),
        "functions":rows,
    })
}

#[derive(Serialize)]
struct WorkerReport<'a> {
    schema_version: u32,
    protocol: &'static str,
    session: &'a Path,
    revision: &'static str,
    requested_functions: Vec<Value>,
    constructed_functions: Vec<&'static str>,
    constructor_exclusions: Vec<Exclusion>,
    configuration_error: Option<String>,
    frames_requested: u64,
    frames_completed: u64,
    run_error: Option<String>,
    finalization_error: Option<String>,
    complete: bool,
    checks: Option<Value>,
}

fn save_report(path: &Path, report: &impl Serialize) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(path)?;
    let mut output = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut output, report)?;
    output.write_all(b"\n")?;
    output.flush()?;
    output.get_ref().sync_all()?;
    Ok(())
}

fn finalize(host: &mut Host) -> Result<()> {
    #[cfg(feature = "replayed-environment")]
    {
        host.finalize_replayed_campaign().map_err(Into::into)
    }
    #[cfg(not(feature = "replayed-environment"))]
    {
        let _ = host;
        anyhow::bail!("replayed-environment feature is not available")
    }
}

pub fn run(bytes: &[u8], path: &Path, names: &[String], destination: &Path) -> Result<bool> {
    let selected = selection::Selection::parse(names, false)?;
    let requested = selected.function_ids();
    ensure!(
        !requested.is_empty(),
        "replay proof requires requested functions"
    );
    let session = Arc::new(Session::from_json(&fs::read_to_string(path)?)?);
    let revision = Revision::from(session.revision);
    ensure!(
        tje_rom::Rom::import(bytes)?.revision() == revision,
        "ROM revision does not match session"
    );
    let mut report = WorkerReport {
        schema_version: 1,
        protocol: "tje-replayed-worker/1",
        session: path,
        revision: revision.name(),
        requested_functions: requested.iter().copied().map(metadata).collect(),
        constructed_functions: Vec::new(),
        constructor_exclusions: Vec::new(),
        configuration_error: None,
        frames_requested: session.frames,
        frames_completed: 0,
        run_error: None,
        finalization_error: None,
        complete: false,
        checks: None,
    };
    let (pending, framed) = selection::pending_bodies();
    let plan = match prepare(&mut NativeFactory, revision, pending, framed, requested) {
        Ok(plan) => plan,
        Err(error) => {
            report.configuration_error = Some(format!("{error:#}"));
            save_report(destination, &report)?;
            return Ok(false);
        }
    };
    report.constructed_functions = plan
        .supported
        .iter()
        .map(|id| id.metadata().inventory_id)
        .collect();
    report.constructor_exclusions = plan.exclusions;
    let Some(registry) = plan.registry else {
        save_report(destination, &report)?;
        return Ok(false);
    };
    let mut host = runner::host_with_registry(bytes, Mode::Check, revision, false, registry)?;
    host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)?;
    host.set_wait_input_poller(Arc::new(runner::SessionInputs::new(Arc::clone(&session))));
    for (frame, pads) in session.inputs().enumerate() {
        if (frame as u64) < host.frame_count() {
            continue;
        }
        runner::apply_recipe(&mut host, &session, frame as u64);
        if let Err(error) = host.run_frame(runner::inputs(pads)) {
            report.run_error = Some(error.to_string());
            break;
        }
        // The exact Host replay packet, not the generic strict mismatch string,
        // is serialized at the actual finalization boundary below.
        host.take_session_observations();
    }
    host.finish_future_frame_provenance();
    report.finalization_error = finalize(&mut host).err().map(|error| format!("{error:#}"));
    report.frames_completed = host.frame_count();
    report.checks = Some(serde_json::to_value(&host.checks)?);
    let evidence = report
        .checks
        .as_ref()
        .and_then(|checks| checks.get("replayed_environment"));
    report.complete = report.constructor_exclusions.is_empty()
        && report.run_error.is_none()
        && report.finalization_error.is_none()
        && report.frames_completed == report.frames_requested
        && evidence.is_some_and(|evidence| {
            evidence.get("schema_version").and_then(Value::as_u64) == Some(1)
                && evidence.get("finalized").and_then(Value::as_bool) == Some(true)
                && evidence.get("complete").and_then(Value::as_bool) == Some(true)
        });
    save_report(destination, &report)
        .with_context(|| format!("preserving replay result {}", destination.display()))?;
    Ok(report.complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_game::ids;

    #[derive(Debug)]
    struct StubRegistry(Vec<FunctionId>);
    struct StubFactory {
        unsupported: Vec<FunctionId>,
        calls: Vec<Vec<FunctionId>>,
        invalid: bool,
    }
    impl RegistryFactory for StubFactory {
        type Registry = StubRegistry;
        fn construct(
            &mut self,
            _revision: Revision,
            _pending: &'static [Translation],
            _framed: &'static [FramedTranslation],
            functions: &[FunctionId],
        ) -> std::result::Result<StubRegistry, ConstructionError> {
            self.calls.push(functions.to_vec());
            if self.invalid {
                return Err(ConstructionError::Invalid("inconsistent metadata".into()));
            }
            if functions.iter().any(|id| self.unsupported.contains(id)) {
                return Err(ConstructionError::Unsupported(
                    "labelled unsupported fixture".into(),
                ));
            }
            Ok(StubRegistry(functions.to_vec()))
        }
    }

    #[test]
    fn stub_registry_preflights_each_original_root_and_preserves_exclusions() {
        let mut factory = StubFactory {
            unsupported: vec![ids::ERROR_TRAP],
            calls: Vec::new(),
            invalid: false,
        };
        let roots = [ids::ABS, ids::ERROR_TRAP];
        let plan = prepare(&mut factory, Revision::Rev02, &[], &[], &roots).unwrap();
        assert_eq!(plan.supported, [ids::ABS]);
        assert_eq!(plan.exclusions.len(), 1);
        assert_eq!(
            plan.exclusions[0].inventory_id,
            ids::ERROR_TRAP.metadata().inventory_id
        );
        assert_eq!(plan.registry.unwrap().0, [ids::ABS]);
        assert_eq!(
            factory.calls,
            [vec![ids::ABS], vec![ids::ERROR_TRAP], vec![ids::ABS]]
        );
    }

    #[test]
    fn all_unsupported_never_turns_an_empty_filter_into_all_roots() {
        let mut factory = StubFactory {
            unsupported: vec![ids::ERROR_TRAP],
            calls: Vec::new(),
            invalid: false,
        };
        let plan = prepare(&mut factory, Revision::Rev00, &[], &[], &[ids::ERROR_TRAP]).unwrap();
        assert!(plan.registry.is_none());
        assert!(plan.supported.is_empty());
        assert_eq!(plan.exclusions.len(), 1);
        assert_eq!(factory.calls.len(), 1);
    }

    #[test]
    fn invalid_metadata_is_fatal_not_a_supported_subset() {
        let mut factory = StubFactory {
            unsupported: Vec::new(),
            calls: Vec::new(),
            invalid: true,
        };
        assert!(prepare(&mut factory, Revision::Rev02, &[], &[], &[ids::ABS]).is_err());
    }

    #[cfg(not(feature = "replayed-environment"))]
    #[test]
    fn missing_real_envelope_never_falls_back_to_an_ordinary_registry() {
        let error = NativeFactory
            .construct(Revision::Rev02, &[], &[], &[ids::ABS])
            .err()
            .expect("real envelope feature is required");
        assert!(matches!(error, ConstructionError::Unsupported(_)));
        assert_eq!(
            catalogue()["replayed_environment_available"].as_bool(),
            Some(false)
        );
    }
}
