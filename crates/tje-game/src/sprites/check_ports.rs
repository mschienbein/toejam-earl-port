//! Opt-in, ROM-backed port-writer proofs on explicitly seeded live hardware.
//! Each fixture starts at a genuinely settled original CPU boundary. FIFO-full
//! and non-halting VRAM-copy DMA remain present at entry; no readiness is patched.
use std::{env, fs, path::PathBuf};
use tje_game::{FunctionId, Platform, execution::ExecutionClass, ids};
use tje_host::{Host, input::InputScript};
use tje_mem::{Platform as MemoryPlatform, ram};

const WRITERS: &[FunctionId] = &[
    ids::SELECT_SPRITE_CHAIN_START,
    ids::TERMINATE_SPRITE_CHAIN_AT_HEAD,
    ids::TERMINATE_PRIMARY_SPRITE_CHAIN,
    ids::SELECT_PRIMARY_VIEWPORT_SPRITES,
    ids::SELECT_SECONDARY_VIEWPORT_SPRITES,
    ids::HIDE_FLAGGED_PLAYER_SPRITES,
];

#[derive(Clone, Copy)]
enum Busy {
    FullFifo,
    VramCopyDma,
    FullFifoAndVramCopyDma,
}
impl Busy {
    fn name(self) -> &'static str {
        match self {
            Self::FullFifo => "full FIFO",
            Self::VramCopyDma => "pending non-halting VRAM-copy DMA",
            Self::FullFifoAndVramCopyDma => "full FIFO and pending VRAM-copy DMA",
        }
    }
    fn fifo(self) -> bool {
        matches!(self, Self::FullFifo | Self::FullFifoAndVramCopyDma)
    }
    fn dma(self) -> bool {
        matches!(self, Self::VramCopyDma | Self::FullFifoAndVramCopyDma)
    }
}

fn idle_hardware_boundary(host: &mut Host, script: &InputScript) -> u64 {
    // Only original instructions and ordinary video frames may finish old work.
    // This outer fixture preparation does not change any function-call bound.
    let mut frames = 0;
    loop {
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let status = host.context().read_device_u16(0xc00004);
        if status & 0x0202 == 0x0200 {
            return frames;
        }
        assert!(
            frames < 4,
            "original preparation did not reach empty FIFO/no DMA"
        );
        host.run_frame(script.at(host.frame_count())).unwrap();
        frames += 1;
    }
}

fn seed_busy_hardware(host: &mut Host, busy: Busy) -> u16 {
    let status = {
        let mut context = host.context();
        // Normal 224-line Mode 5, display/VINT/DMA enabled, two-byte VRAM increment.
        // These are authored hardware inputs, not a fabricated function result.
        context.write_vdp_control(0x8174);
        context.write_vdp_control(0x8f02);
        if busy.fifo() {
            // VRAM 0x5000; four writes fill, but do not overflow, the FIFO.
            // This target does not overlap either endpoint of the DMA below.
            context.write_vdp_command(0x5000_0001);
            for word in [0x1111, 0x2222, 0x3333, 0x4444] {
                context.write_vdp_data(word);
            }
        }
        if busy.dma() {
            // Four-byte VRAM-copy DMA, source 0x6000, destination 0x6800.
            // Unlike memory-to-VRAM DMA, this is a legal CPU-ready busy entry.
            for word in [0x9304, 0x9400, 0x9500, 0x9660, 0x97c0] {
                context.write_vdp_control(word);
            }
            context.write_vdp_command(0x6800_0081);
        }
        let status = context.read_device_u16(0xc00004);
        assert_eq!(
            status & 0x0100 != 0,
            busy.fifo(),
            "{} FIFO-full",
            busy.name()
        );
        assert_eq!(
            status & 0x0002 != 0,
            busy.dma(),
            "{} DMA-active",
            busy.name()
        );
        status
    };
    // Never settle here: doing so could erase precisely the busy entry under test.
    assert!(
        host.fixture_boundary_is_settled(),
        "busy seed overtook original CPU work"
    );
    status
}

pub(super) fn fixtures(host: &mut Host, script: &InputScript) {
    if env::var_os("TJE_SPRITE_BUSY_PORTS").is_none() {
        return;
    }
    let evidence = PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").unwrap());
    let revision = host.revision();
    let entity = ram::g_player_info(revision).at(39);
    let mut records = Vec::new();
    for &id in WRITERS {
        if !host.checks.by_function.contains_key(id.metadata().name) {
            continue;
        }
        assert_eq!(
            tje_game::execution_class(id),
            Some(ExecutionClass::Functional)
        );
        let arguments: &[&[u32]] = if id == ids::SELECT_SPRITE_CHAIN_START {
            &[&[0], &[1], &[0xffff_ffff]]
        } else if id == ids::HIDE_FLAGGED_PLAYER_SPRITES {
            &[&[39]]
        } else {
            &[&[]]
        };
        for busy in [
            Busy::FullFifo,
            Busy::VramCopyDma,
            Busy::FullFifoAndVramCopyDma,
        ] {
            for &arguments in arguments {
                let drain_frames = idle_hardware_boundary(host, script);
                let saved_entity: [u8; 128] = std::array::from_fn(|offset| {
                    host.context().read_u8(entity.address() + offset as u32)
                });
                let primary = ram::g_primary_viewport_uploaded_link(revision).read(&host.context());
                let secondary =
                    ram::g_secondary_viewport_uploaded_link(revision).read(&host.context());
                ram::g_primary_viewport_uploaded_link(revision).write(&mut host.context(), 0x80);
                ram::g_secondary_viewport_uploaded_link(revision).write(&mut host.context(), 0xff);
                if id == ids::HIDE_FLAGGED_PLAYER_SPRITES {
                    entity.behavior_flags().write(&mut host.context(), 0x10);
                    entity.sprite_ready().write(&mut host.context(), 1);
                    entity.draw_flags().write(&mut host.context(), 2);
                    entity.sprite_count().write(&mut host.context(), 12);
                    for (index, slot) in [-128, -1, 0, 1, 11, 23, 24, 25, 78, 79, 126, 127]
                        .into_iter()
                        .enumerate()
                    {
                        entity
                            .primary_sprites()
                            .at(index as i32)
                            .write(&mut host.context(), slot);
                    }
                }
                let status = seed_busy_hardware(host, busy);
                let before = host.checks.by_function[id.metadata().name].calls_checked;
                let result = host.check_fixture(id.metadata().address(revision), arguments);
                records.push(serde_json::json!({
                    "function": id.metadata().name, "busy_entry": busy.name(),
                    "arguments": arguments, "entry_status": format!("{status:04X}"),
                    "fifo_full": busy.fifo(), "dma_active": busy.dma(),
                    "entry_dma_mode": format!("{:?}", host.vdp_registers().dma_mode),
                    "entry_dma_length_register": host.vdp_registers().dma_length,
                    "original_drain_video_frames": drain_frames,
                    "settled_after_seeding": true, "result": result.as_ref().map(|_| "matched").map_err(ToString::to_string),
                    "calls_before": before, "calls_after": host.checks.by_function[id.metadata().name].calls_checked,
                }));
                fs::write(
                    evidence.join(format!(
                        "{}-busy-ports.json",
                        revision.name().to_ascii_lowercase().replace(' ', "")
                    )),
                    serde_json::to_vec_pretty(&records).unwrap(),
                )
                .unwrap();
                result.unwrap_or_else(|error| {
                    panic!("{} {}: {error}", id.metadata().name, busy.name())
                });
                assert_eq!(
                    host.checks.by_function[id.metadata().name].calls_checked,
                    before + 1
                );
                assert_eq!(host.checks.mismatches, 0);
                // Only authored fixture RAM is restored. Old hardware work is
                // finished by real original progress before the next fixture.
                for (offset, value) in saved_entity.into_iter().enumerate() {
                    host.context()
                        .write_u8(entity.address() + offset as u32, value);
                }
                ram::g_primary_viewport_uploaded_link(revision).write(&mut host.context(), primary);
                ram::g_secondary_viewport_uploaded_link(revision)
                    .write(&mut host.context(), secondary);
            }
        }
    }
}
