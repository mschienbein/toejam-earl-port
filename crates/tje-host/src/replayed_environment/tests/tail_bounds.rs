//! Authored control-profile boundaries, not any catalogue body's proof.
use super::*;
fn store(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 9);
    Ok(None)
}
static STORE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: store,
}];

#[test]
fn cross_catalogue_bra_entry_interior_and_fallthrough_are_unsealed_tails_not_independent_calls() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for interior in [false, true] {
            let other = ids::ANIMATE_SPLIT_SCREEN_WIPE;
            let target = other.metadata().address(revision) + u32::from(interior) * 2;
            let displacement =
                i16::try_from(i64::from(target) - i64::from(ROOT.metadata().address(revision) + 2))
                    .unwrap();
            let parent = vec![0x6000, displacement as u16];
            let child = vec![0x4e71, 0x13fc, 9, 0x00ff, 0x9002, 0x4e75];
            let mut host = host(
                revision,
                &[],
                STORE,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, parent), (other, child)],
            );
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("tail") || error.to_string().contains("extent"),
                "{error}"
            );
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
            assert!(
                !report
                    .by_function
                    .contains_key(other.metadata().inventory_id),
                "no fake B call/body proof is minted"
            );
        }
        let mut code = vec![0x4e71; ROOT.metadata().size(revision) as usize / 2];
        code.extend([0x13fc, 9, 0x00ff, 0x9002, 0x4e75]);
        let mut host = host(
            revision,
            &[],
            STORE,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, code)],
        );
        let error = finish_one(&mut host).unwrap_err();
        assert!(
            error.to_string().contains("tail") || error.to_string().contains("extent"),
            "{error}"
        );
        assert_eq!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .completed_proofs,
            0
        );
    }
}
#[test]
fn internal_bra_and_ccr_only_instructions_remain_ordinary_semantic_operations() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for prefix in [
            vec![0x6002, 0x4e71],
            vec![0x003c, 0],
            vec![0x023c, 0xff],
            vec![0x0a3c, 0],
            vec![0x44fc, 0],
        ] {
            let code = [prefix, vec![0x13fc, 9, 0x00ff, 0x9002, 0x4e75]].concat();
            let mut host = host(
                revision,
                &[],
                STORE,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            finish_one(&mut host).unwrap();
            host.finalize_replayed_campaign().unwrap();
            assert_eq!(
                host.checks
                    .replayed_environment
                    .as_ref()
                    .unwrap()
                    .completed_proofs,
                1
            );
        }
    }
}
#[test]
fn idempotent_full_sr_operations_require_control_profile_even_without_endpoint_delta() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for control in [[0x007c, 0], [0x027c, 0xffff], [0x0a7c, 0], [0x46fc, 0x2700]] {
            let code = [control.to_vec(), vec![0x13fc, 9, 0x00ff, 0x9002, 0x4e75]].concat();
            let mut host = host(
                revision,
                &[],
                STORE,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("privileged instruction class"),
                "{error}"
            );
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
            assert_eq!(
                report.last_unsupported.as_ref().unwrap().kind,
                UnsupportedKind::TypedControlProfile
            );
        }
    }
}
