//! Strengthen an actual C entry's proof where its caller consumes compiler residue.
use std::{env, fs, path::PathBuf};
use tje_game::{Context, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

fn strict_indexes(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(tje_game::call(context, ids::SET_RAM_INDEXES, &[]).map(|_| None)?)
}

pub fn prove_indexes(root: &std::path::Path, evidence: &std::path::Path, script: &InputScript) {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let bytes = fs::read(root.join("roms").join(format!("{key}.bin"))).unwrap();
        let mut symbol = ids::SET_RAM_INDEXES.metadata().clone();
        symbol.abi.strict_registers = true;
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_contract(symbol, true, strict_indexes)
            .unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        for high in [0u32, 0x1234_0000, 0xffff_0000] {
            for ccr in 0..32u16 {
                let control = host.context().status_register() & !0x1f;
                host.context().set_status_register(control | ccr);
                host.context().set_data_register(0, high | 0xabcd);
                host.context().set_data_register(1, 0x1020_3040);
                host.context().set_address_register(0, 0xffe800);
                host.context().set_address_register(1, 0xffe900);
                host.check_fixture(ids::SET_RAM_INDEXES.metadata().address(revision), &[])
                    .unwrap_or_else(|error| {
                        panic!("strict {key} SetRAMIndexes high={high:08x} ccr={ccr:02x}: {error}")
                    });
            }
        }
        assert_eq!(host.checks.mismatches, 0);
        assert_eq!(host.checks.by_function["SetRAMIndexes"].calls_checked, 96);
        let result = serde_json::json!({"revision":key, "strict_registers":true,
            "cases":"three D0 upper words times all32 CCR combinations; D1/A1 preservation",
            "boundary":{"ticks":boundary.ticks,"master_clocks":boundary.master_clocks,
                "start_pc":boundary.start_pc,"end_pc":boundary.end_pc,
                "handlers_completed":boundary.handlers_completed}, "checks":host.checks});
        fs::write(
            evidence.join(format!("{key}-strict-indexes.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("STRICT_INDEXES {result}");
    }
}

pub fn requested() -> bool {
    env::var_os("TJE_SPRITE_STRICT_INDEXES").is_some()
}

pub fn paths() -> (PathBuf, PathBuf) {
    (
        PathBuf::from(env::var_os("TJE_PORT_ROOT").expect("TJE_PORT_ROOT")),
        PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").expect("TJE_SPRITE_EVIDENCE")),
    )
}
