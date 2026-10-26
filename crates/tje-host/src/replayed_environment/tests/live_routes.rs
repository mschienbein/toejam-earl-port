//! Authored preflight controls only, no replacement/body proof authority.
use super::*;
fn effect(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u16(VALUE, 0x1234);
    context.write_device_u16(0xc00004, 0x8f02);
    context.set_data_register(0, 0xfeedbeef);
    Ok(None)
}
static REPLAY_BODY: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: effect,
}];
#[test]
fn replace_root_external_and_unhooked_replay_rust_preflight_has_zero_effects() {
    for route in 0..4 {
        let mut host = crate::fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
        host.mode = Mode::Replace;
        host.registry = HookRegistry::empty(Revision::Rev02);
        let before_cpu = host.emulator.hybrid_cpu().hybrid_state_words();
        let before_ram = host
            .emulator
            .hybrid_bus()
            .memory
            .hybrid_raw_main_ram()
            .to_vec();
        let before_vdp =
            bincode::encode_to_vec(&host.emulator.hybrid_bus().vdp, bincode::config::standard())
                .unwrap();
        let before_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let dispatcher = tje_game::Dispatcher::hybrid_with_frames(&[], REPLAY_BODY);
        let frame;
        let result = {
            let mut context = host.context();
            frame = context.frame_state();
            let result = match route {
                0 => dispatcher.call_void(&mut context, ROOT, &[]),
                1 => tje_game::call_frame::call_at_void(
                    &mut context,
                    dispatcher,
                    ROOT,
                    &[],
                    tje_game::call_frame::CallSite::new(0x100),
                ),
                2 => tje_game::TranslatedEntry::Framed(&REPLAY_BODY[0])
                    .invoke_entry(&mut context, &[])
                    .map(|_| ()),
                3 => {
                    let registry = HookRegistry::proof_replayed_environment(
                        Revision::Rev02,
                        &[],
                        REPLAY_BODY,
                        &[ROOT],
                    )
                    .unwrap();
                    let hook = registry
                        .at(ROOT.metadata().address(Revision::Rev02))
                        .unwrap();
                    let error = hook.execute(&mut context).unwrap_err();
                    assert!(error.to_string().contains("check-only"), "{error}");
                    Err(tje_game::Error::Platform(error.to_string()))
                }
                _ => unreachable!(),
            };
            assert_eq!(context.frame_state(), frame);
            result
        };
        assert!(result.unwrap_err().to_string().contains("check-only"));
        assert_eq!(host.emulator.hybrid_cpu().hybrid_state_words(), before_cpu);
        assert_eq!(
            host.emulator.hybrid_bus().memory.hybrid_raw_main_ram(),
            before_ram
        );
        assert_eq!(
            bincode::encode_to_vec(&host.emulator.hybrid_bus().vdp, bincode::config::standard())
                .unwrap(),
            before_vdp
        );
        assert_eq!(
            host.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            before_clock
        );
        assert_eq!(host.frame, 0);
    }
}
