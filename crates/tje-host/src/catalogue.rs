//! Complete public function metadata is generated once in tje-game. Both host
//! revisions select addresses there; no ROM contents or decomp checkout needed.
use tje_game::{FunctionMetadata, ids};
use tje_rom::Revision;

pub const RNG_STATE: u32 = 0xFF91E2;
pub const FRAME_COUNTER: u32 = 0xFF800E;
pub const STACK_BOTTOM: u32 = 0xFF0000;
pub const STACK_TOP: u32 = 0xFF8000;

pub type FunctionSymbol = FunctionMetadata;
#[derive(Debug, Clone, Copy)]
pub struct FunctionCatalogue {
    revision: Revision,
}
impl FunctionCatalogue {
    pub const fn new(revision: Revision) -> Self {
        Self { revision }
    }
    pub const fn revision(self) -> Revision {
        self.revision
    }
    pub fn at(&self, address: u32) -> Option<&'static FunctionSymbol> {
        tje_game::function_at(self.revision, address).map(|id| id.metadata())
    }
    pub fn named(&self, name: &str) -> Option<&'static FunctionSymbol> {
        tje_game::functions().find(|function| function.name == name)
    }
}

/// Instruction-boundary observation points. The three offsets are labels inside
/// G/src/core/startup_and_main_loop.c:FurtherInitialisation, whose body has the
/// same size and address-only relocations in both revisions (inventory evidence).
#[derive(Debug, Clone, Copy)]
pub struct RevisionSymbols {
    pub intro: u32,
    pub title: u32,
    pub new_game_menu: u32,
    pub main_loop: u32,
    pub main_wait: u32,
    pub idle_random_return: u32,
    pub random: u32,
}
impl RevisionSymbols {
    pub fn for_revision(revision: Revision) -> Self {
        let startup = ids::FURTHER_INITIALISATION.metadata().address(revision);
        Self {
            intro: ids::SEGA_AND_INTRO_VIDEO_ROUTINE
                .metadata()
                .address(revision),
            title: ids::TITLE_SCREEN_VIDEO.metadata().address(revision),
            new_game_menu: ids::NEW_GAME_MAIN_MENU.metadata().address(revision),
            main_loop: startup + 0x260,
            main_wait: startup + 0x2f4,
            idle_random_return: startup + 0x2fc,
            random: ids::GET_RANDOM_NUMBER.metadata().address(revision),
        }
    }
}
