use tje_game::ids;

#[test]
fn centered_cutscene_text_array_formals_are_full_pointer_slots() {
    // G/src/cutscenes/cutscene_text.c:62-64,91-93 declares char text[].
    // Both matching listings load MOVE.L 32(SP),D3 after a 28-byte save,
    // hence the complete pointer at entry SP+4, never a char at SP+7.
    let first_argument = concat!(
        "\"arguments\":[{\"declared_width_bits\":32,",
        "\"entry_sp_slot_offset\":4,\"entry_sp_value_offset\":4,",
        "\"name\":\"text\",\"slot_bytes\":4,",
        "\"source_declaration\":\"char text[]\",\"type\":\"char *\"}",
    );
    for (id, signature) in [
        (
            ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
            "void CutsceneQueueCenteredRemappedTextLine(char * text, short length, short tile_base)",
        ),
        (
            ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
            "void CutsceneQueueCenteredOriginalPaletteTextLine(char * text, short length, short tile_base)",
        ),
    ] {
        let function = id.metadata();
        assert_eq!(function.abi.kind, "mcc68k_c");
        assert_eq!(function.abi.argument_count, 3);
        assert_eq!(function.abi.signature, signature);
        assert!(function.abi.details_json.contains(first_argument));
        assert!(
            tje_game::cutscenes::TRANSLATIONS
                .iter()
                .any(|translation| translation.id == id),
        );
    }
}
