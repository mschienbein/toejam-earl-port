//! Strengthen an actual C entry's proof where its caller consumes compiler residue.
use std::sync::atomic::{AtomicU16, Ordering};
use std::{env, fs, path::PathBuf};
use tje_game::{Context, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Ptr, RenderPiece, Revision, ram};

fn strict_indexes(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(tje_game::call_void(context, ids::SET_RAM_INDEXES, &[]).map(|_| None)?)
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

fn strict_sprite(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let arguments = [context.argument_u32(0), context.argument_u32(1)];
    Ok(tje_game::call_void(context, ids::SET_SPRITE, &arguments).map(|_| None)?)
}

pub fn prove_sprite(root: &std::path::Path, evidence: &std::path::Path, script: &InputScript) {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let bytes = fs::read(root.join("roms").join(format!("{key}.bin"))).unwrap();
        let mut symbol = ids::SET_SPRITE.metadata().clone();
        symbol.abi.strict_registers = true;
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_contract(symbol, true, strict_sprite)
            .unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        for player in 0..2 {
            let entity = ram::g_player_info(revision).at(player);
            entity
                .entity_index()
                .write(&mut host.context(), player as i8);
            for state in 0..=255u32 {
                for upper in [0, 0x1234_5600, 0xffff_ff00] {
                    for ccr in 0..32u16 {
                        for index in 0..8 {
                            host.context().set_data_register(
                                index,
                                0x89ab_cde0u32.wrapping_add(index as u32),
                            );
                        }
                        for index in 0..7 {
                            host.context().set_address_register(
                                index,
                                0x1020_3000u32.wrapping_add(index as u32 * 0x100),
                            );
                        }
                        host.context().set_data_register(0, upper | 0x5a);
                        host.context().set_status_register(0x2700 | ccr);
                        if let Err(error) = host.check_fixture(
                            ids::SET_SPRITE.metadata().address(revision),
                            &[entity.address(), state],
                        ) {
                            let failure = serde_json::json!({
                                "revision":key, "player":player, "state":state,
                                "d0_upper24":upper, "entry_ccr":ccr,
                                "error":error.to_string(), "checks":host.checks,
                            });
                            fs::write(
                                evidence.join(format!("{key}-strict-sprite-failed.json")),
                                serde_json::to_vec_pretty(&failure).unwrap(),
                            )
                            .unwrap();
                            panic!(
                                "strict {key} SetSprite player={player} state={state:02x} upper={upper:08x} ccr={ccr:02x}: {error}"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(host.checks.by_function["SetSprite"].calls_checked, 49_152);
        let result = serde_json::json!({
            "revision":key, "strict_registers":true,
            "cases":"all256 states x both players x three D0 upper24 patterns x all32 CCR",
            "boundary":{"ticks":boundary.ticks,"master_clocks":boundary.master_clocks,
                "start_pc":boundary.start_pc,"end_pc":boundary.end_pc,
                "handlers_completed":boundary.handlers_completed},
            "checks":host.checks,
        });
        fs::write(
            evidence.join(format!("{key}-strict-sprite.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("STRICT_SPRITE {result}");
    }
}

pub fn sprite_requested() -> bool {
    env::var_os("TJE_SPRITE_STRICT_SPRITE").is_some()
}

static QUEUE_RETURN_SR: AtomicU16 = AtomicU16::new(0);

fn observe_queue_return(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let arguments = [context.argument_u32(0), context.argument_u32(1)];
    let result = tje_game::call(context, ids::QUEUE_SPRITE_PIECE_TILES, &arguments)?;
    QUEUE_RETURN_SR.store(context.status_register(), Ordering::Relaxed);
    Ok(Some(result))
}

/// Check the actual native producer's full SR against the real original return.
/// Ordinary C ABI remains unchanged: the diagnostic observes SR, never patches it.
pub fn prove_queue_ccr(root: &std::path::Path, evidence: &std::path::Path, script: &InputScript) {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let bytes = fs::read(root.join("roms").join(format!("{key}.bin"))).unwrap();
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_contract(
                ids::QUEUE_SPRITE_PIECE_TILES.metadata().clone(),
                true,
                observe_queue_return,
            )
            .unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
        let piece = Address::<RenderPiece>::new(buffer + 7680);
        let stream = buffer + 7800;
        piece
            .tile_data()
            .write(&mut host.context(), Ptr::new(stream));
        let mut cases = 0;
        for (width, height) in [(0, 1), (1, 1), (255, 255)] {
            piece.width().write(&mut host.context(), width);
            piece.height().write(&mut host.context(), height);
            for format in [0, -128] {
                piece.tile_format().write(&mut host.context(), format);
                let data: &[u8] = if format < 0 {
                    &[0xf8, 0x12, 0xf8, 0x34, 0xf8, 0x56, 0xf8, 0x78]
                } else {
                    &[0xe0, 0x12]
                };
                for (index, value) in data.iter().copied().enumerate() {
                    host.context().write_u8(stream + index as u32, value);
                }
                for queue_index in [0, 254, 255] {
                    for pending in [0, 255] {
                        for ccr in 0..32u16 {
                            ram::g_sprite_tile_buffer_offset(revision)
                                .write(&mut host.context(), 0);
                            ram::g_sprite_tile_upload_queue_word_count(revision)
                                .write(&mut host.context(), queue_index);
                            ram::g_sprite_graphics_upload_pending(revision)
                                .write(&mut host.context(), pending);
                            host.context().set_status_register(0x2700 | ccr);
                            host.check_call(
                                ids::QUEUE_SPRITE_PIECE_TILES.metadata().address(revision),
                                &[piece.address(), 640],
                            )
                            .unwrap();
                            let actual_original = host.cpu().status_register();
                            let actual_native = QUEUE_RETURN_SR.load(Ordering::Relaxed);
                            assert_eq!(
                                actual_native, actual_original,
                                "{key} width={width} height={height} format={format} queue={queue_index} pending={pending} ccr={ccr}"
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 1152);
        assert_eq!(host.checks.mismatches, 0);
        let result = serde_json::json!({"revision":key,"cases":cases,"full_original_native_sr":"exact",
            "abi_unchanged":true,"output_substitution":false,"checks":host.checks});
        fs::write(
            evidence.join(format!("{key}-queue-return-sr.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("QUEUE_RETURN_SR {result}");
    }
}

pub fn queue_ccr_requested() -> bool {
    env::var_os("TJE_SPRITE_QUEUE_CCR").is_some()
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
