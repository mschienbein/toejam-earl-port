use crate::{Error, HEIGHT, Result, WIDTH, packet::View};
use jgenesis_common::frontend::Color;
use tje_extended_view::{IndexedPixel, MapRenderer, TerrainSource, resources::color_word};
use tje_four_player::Character;
use tje_mem::AddressSpace;
use tje_simulation::PublicationStamp;
use tje_world::{LevelGeometry, LevelId};

pub struct GeometrySource<'a> {
    pub geometry: &'a LevelGeometry,
    pub level: LevelId,
}
impl TerrainSource for GeometrySource<'_> {
    fn cell(&self, column: i32, row: i32) -> Option<u16> {
        self.geometry.cell(column, row).map(|cell| cell.word())
    }
    fn exterior(&self) -> Option<u16> {
        matches!(self.level.get(), 1 | 26).then_some(82)
    }
}
/// Implementations may borrow a committed aggregate under this stamp. A renderer
/// never calls its mutation/query services or observes next-phase geometry.
pub trait FrameSource {
    fn stamp(&self) -> PublicationStamp;
    fn revision(&self) -> tje_rom::Revision;
    fn rom(&self) -> &[u8];
    fn vram(&self) -> &[u8];
    fn terrain(&self, view: View, output: &mut [IndexedPixel]) -> Result<()>;
    fn color(&self, index: usize, character: Option<Character>, modifier: i8) -> Color;
    fn map_color(&self, level: LevelId, position: [i32; 2]) -> Result<Color>;
}
/// Borrowed source adapter for stock-sized, expanded and designed LevelGeometry.
/// The four active levels and device patterns/palettes are never copied per frame.
pub struct WorldSource<'a> {
    stamp: PublicationStamp,
    memory: &'a AddressSpace,
    renderer: MapRenderer<'a>,
    palettes: [[[Color; 64]; 3]; 3],
    levels: &'a [(LevelId, &'a LevelGeometry)],
    vram: &'a [u8],
}
impl<'a> WorldSource<'a> {
    pub fn new(
        stamp: PublicationStamp,
        memory: &'a AddressSpace,
        vram: &'a [u8],
        cram: &'a [u16],
        levels: &'a [(LevelId, &'a LevelGeometry)],
    ) -> Result<Self> {
        if cram.len() != 64
            || levels.len() > 4
            || levels.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(Error::Invalid(
                "invalid published palette/ordered active level set",
            ));
        }
        let palettes = std::array::from_fn(|variant| {
            std::array::from_fn(|modifier| {
                std::array::from_fn(|index| {
                    let variant = [
                        tje_four_player::PaletteVariant::Original,
                        tje_four_player::PaletteVariant::RotateForward,
                        tje_four_player::PaletteVariant::RotateBackward,
                    ][variant];
                    color_word(
                        tje_four_player::palette::variant_word(
                            cram[index],
                            index & 15 == 0,
                            variant,
                        ),
                        modifier as i8 - 1,
                    )
                })
            })
        });
        Ok(Self {
            stamp,
            memory,
            renderer: MapRenderer::new(memory, vram)?,
            palettes,
            levels,
            vram,
        })
    }
    fn geometry(&self, level: LevelId) -> Result<GeometrySource<'_>> {
        let index = self
            .levels
            .binary_search_by_key(&level, |(id, _)| *id)
            .map_err(|_| Error::Invalid("published level is not borrowed by the frame source"))?;
        Ok(GeometrySource {
            geometry: self.levels[index].1,
            level,
        })
    }
}
impl FrameSource for WorldSource<'_> {
    fn stamp(&self) -> PublicationStamp {
        self.stamp
    }
    fn revision(&self) -> tje_rom::Revision {
        self.memory.revision()
    }
    fn rom(&self) -> &[u8] {
        self.memory.rom()
    }
    fn vram(&self) -> &[u8] {
        self.vram
    }
    fn terrain(&self, view: View, output: &mut [IndexedPixel]) -> Result<()> {
        let View::World {
            level,
            origin,
            attributes,
            starfield,
            ..
        } = view
        else {
            output.fill(IndexedPixel::default());
            return Ok(());
        };
        self.renderer.render_background_from(
            &self.geometry(level)?,
            origin,
            [origin[0] / 2, origin[1] / 2],
            [WIDTH, HEIGHT],
            attributes,
            starfield,
            output,
        )?;
        Ok(())
    }
    fn color(&self, index: usize, character: Option<Character>, modifier: i8) -> Color {
        let variant = match character.map(|hero| hero.palette) {
            None | Some(tje_four_player::PaletteVariant::Original) => 0,
            Some(tje_four_player::PaletteVariant::RotateForward) => 1,
            Some(tje_four_player::PaletteVariant::RotateBackward) => 2,
        };
        self.palettes[variant][(modifier.clamp(-1, 1) + 1) as usize][index & 63]
    }
    fn map_color(&self, level: LevelId, position: [i32; 2]) -> Result<Color> {
        // Overview uses the actual decoded center pixel, not invented terrain IDs.
        let mut sample = [IndexedPixel::default()];
        self.renderer.render_terrain_from(
            &self.geometry(level)?,
            position,
            [1, 1],
            0,
            &mut sample,
        )?;
        Ok(self.color(sample[0].palette_index(), None, 0))
    }
}
