use tje_mem::{Address, ObjectState, Revision, ram};

/// MCC68K's ASL.W #7 followed by ADDA.W for gPlayerInfo[short_index].
/// Used only where the matching caller emits this word-width scale. In
/// particular, relocation's (player_number ^ 1) partner uses ASL.L instead.
/// Sources: G/build/rev02/asm/c_active_present_falling_and_respawn.s:114-117,
/// 1881-1885,2127-2130; c_earthling_damage_reactions.s:39-43,244-248,
/// 317-320,541-543,695-698,1078-1081,1219-1222; c_mole.s:185-188;
/// c_hottub_ability.s:12-15; c_ability_helpers.s:56-59.
pub(super) const fn player_by_word_index(revision: Revision, index: i32) -> Address<ObjectState> {
    let displacement = (index as i16).wrapping_shl(7);
    ram::g_player_info(revision)
        .byte_offset(displacement as i32)
        .cast::<ObjectState>()
}

#[cfg(test)]
mod tests {
    use super::player_by_word_index;
    use tje_mem::{Revision, ram};

    #[test]
    fn every_short_index_matches_asl_word_then_sign_extended_adda_word() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let base = ram::g_player_info(revision).address();
            for bits in 0..=u16::MAX {
                let scaled = (u32::from(bits) << 7) & 0xffff;
                let signed = if scaled & 0x8000 != 0 {
                    scaled as i32 - 0x10000
                } else {
                    scaled as i32
                };
                assert_eq!(
                    player_by_word_index(revision, i32::from(bits as i16)).address(),
                    base.wrapping_add(signed as u32)
                );
            }
            assert_eq!(player_by_word_index(revision, 512).address(), base);
            assert_eq!(player_by_word_index(revision, -512).address(), base);
            assert_eq!(
                player_by_word_index(revision, 513).address(),
                ram::g_player_info(revision).at(1).address()
            );
            assert_eq!(
                player_by_word_index(revision, -1).address(),
                base.wrapping_sub(128)
            );
        }
    }
}
