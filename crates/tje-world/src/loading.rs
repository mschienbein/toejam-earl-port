//! Resumable imports for original/generated, authored special and DesignedV1
//! geometry. Candidates remain private until their complete content digest,
//! population and extents validate. A cancelled import never changes a floor.
use crate::{
    Anchors, Cell, Dimensions, Error, LevelGeometry, LevelId, Resources, Result, WorldConfig,
    generation::{GeneratedLevel, GenerationSpec},
    population::Population,
    random::StageRng,
};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SpecialFloor {
    Zero,
    One,
    Final,
}
impl SpecialFloor {
    pub const fn level(self) -> LevelId {
        match self {
            Self::Zero => LevelId::new(0).unwrap(),
            Self::One => LevelId::new(1).unwrap(),
            Self::Final => LevelId::new(26).unwrap(),
        }
    }
    /// Original falling below floor zero returns to the highest visited floor;
    /// floors one and 26 have water exteriors, not coordinate wrapping.
    pub fn fall_destination(source: LevelId, highest: LevelId) -> Result<LevelId> {
        source.validate()?;
        highest.validate()?;
        if source.get() > 26 || highest.get() > 26 || highest.get() == 0 {
            return Err(Error::Bounds("campaign fall/highest floor"));
        }
        Ok(if source.get() == 0 {
            highest
        } else {
            LevelId::new(source.get() - 1).expect("campaign floor")
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum ImportKind {
    Stock,
    Special(SpecialFloor),
    Designed { course_identity: [u8; 32] },
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum FloorSpec {
    Expanded(GenerationSpec),
    Imported(ImportedFloorSpec),
}
impl FloorSpec {
    pub fn level(&self) -> LevelId {
        match self {
            Self::Expanded(spec) => spec.level,
            Self::Imported(spec) => spec.level,
        }
    }
    pub fn config(&self) -> &WorldConfig {
        match self {
            Self::Expanded(spec) => &spec.config,
            Self::Imported(spec) => &spec.config,
        }
    }
    pub fn dimensions(&self) -> Dimensions {
        match self {
            Self::Expanded(spec) => spec.config.dimensions_for(spec.level),
            Self::Imported(spec) => spec.dimensions,
        }
    }
    pub fn identity(&self) -> [u8; 32] {
        match self {
            Self::Expanded(spec) => spec.identity(),
            Self::Imported(spec) => spec.identity(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Expanded(spec) => spec.validate(),
            Self::Imported(spec) => spec.validate(),
        }
    }
}

/// Compact immutable provenance; source populations are not duplicated alongside
/// their mutable persistent records after loading completes.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ImportedFloorSpec {
    pub config: WorldConfig,
    pub level: LevelId,
    pub kind: ImportKind,
    pub dimensions: Dimensions,
    pub source_digest: [u8; 32],
    pub definitions_digest: [u8; 32],
}
impl ImportedFloorSpec {
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        self.level.validate()?;
        self.dimensions.validate()?;
        match self.kind {
            ImportKind::Stock
                if !self.level.ordinary() || self.dimensions != Dimensions::ORIGINAL =>
            {
                return Err(Error::Bounds("stock imported floor"));
            }
            ImportKind::Special(kind)
                if self.level != kind.level() || self.dimensions != Dimensions::ORIGINAL =>
            {
                return Err(Error::Bounds(
                    "authored special floor must retain original dimensions",
                ));
            }
            _ => (),
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-world/import/1\0");
        hash.update(self.config.identity());
        hash.update(self.level.get().to_be_bytes());
        hash.update(self.dimensions.width().to_be_bytes());
        hash.update(self.dimensions.height().to_be_bytes());
        match self.kind {
            ImportKind::Stock => hash.update([0]),
            ImportKind::Special(kind) => hash.update([
                1,
                match kind {
                    SpecialFloor::Zero => 0,
                    SpecialFloor::One => 1,
                    SpecialFloor::Final => 26,
                },
            ]),
            ImportKind::Designed { course_identity } => {
                hash.update([2]);
                hash.update(course_identity);
            }
        }
        hash.update(self.definitions_digest);
        hash.update(self.source_digest);
        hash.finalize().into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct SourceCell {
    pub cell: Cell,
    pub directions: u8,
    pub marked: bool,
    pub alternate: bool,
}
impl SourceCell {
    pub fn validate(self) -> Result<()> {
        self.cell.validate()?;
        if self.directions > 15 || (self.directions != 0 && !self.marked) {
            return Err(Error::State("imported path direction/mark"));
        }
        Ok(())
    }
}
pub trait CellSource {
    fn cell(&self, index: usize) -> Result<SourceCell>;
}
impl CellSource for LevelGeometry {
    fn cell(&self, index: usize) -> Result<SourceCell> {
        let cell = *self
            .cells()
            .get(index)
            .ok_or(Error::Bounds("imported cell index"))?;
        let directions = self.path_state()[index];
        Ok(SourceCell {
            cell,
            directions,
            marked: directions != 0,
            alternate: false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ImportSpec {
    pub config: WorldConfig,
    pub level: LevelId,
    pub kind: ImportKind,
    pub dimensions: Dimensions,
    pub anchors: Anchors,
    pub population: Population,
    pub source_digest: [u8; 32],
}
impl ImportSpec {
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        self.level.validate()?;
        self.dimensions.validate()?;
        self.anchors.validate(self.dimensions)?;
        self.population.validate(self.dimensions)?;
        match self.kind {
            ImportKind::Stock
                if !self.level.ordinary() || self.dimensions != Dimensions::ORIGINAL =>
            {
                return Err(Error::Bounds("stock imported floor"));
            }
            ImportKind::Special(kind)
                if self.level != kind.level() || self.dimensions != Dimensions::ORIGINAL =>
            {
                return Err(Error::Bounds(
                    "authored special floor must retain original dimensions",
                ));
            }
            _ => (),
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        self.provenance().identity()
    }
    fn provenance(&self) -> ImportedFloorSpec {
        struct HashWriter(Sha256);
        impl std::io::Write for HashWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = HashWriter(Sha256::new());
        writer.0.update(b"tje-world/import-definitions/1\0");
        bincode::encode_into_std_write(&self.anchors, &mut writer, crate::world::codec())
            .expect("infallible hash sink");
        bincode::encode_into_std_write(&self.population, &mut writer, crate::world::codec())
            .expect("infallible hash sink");
        ImportedFloorSpec {
            config: self.config.clone(),
            level: self.level,
            kind: self.kind.clone(),
            dimensions: self.dimensions,
            source_digest: self.source_digest,
            definitions_digest: writer.0.finalize().into(),
        }
    }
    pub fn content_digest(dimensions: Dimensions, source: &impl CellSource) -> Result<[u8; 32]> {
        dimensions.validate()?;
        let mut digest = initial_digest(dimensions);
        for index in 0..dimensions.area() {
            let cell = source.cell(index)?;
            cell.validate()?;
            digest = append_digest(digest, cell);
        }
        Ok(digest)
    }
}
fn initial_digest(dimensions: Dimensions) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"tje-world/import-cells/1\0");
    hash.update(dimensions.width().to_be_bytes());
    hash.update(dimensions.height().to_be_bytes());
    hash.finalize().into()
}
fn append_digest(previous: [u8; 32], cell: SourceCell) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(previous);
    hash.update(cell.cell.word().to_be_bytes());
    hash.update([
        cell.directions,
        u8::from(cell.marked),
        u8::from(cell.alternate),
    ]);
    hash.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ImportProgress {
    Loading,
    Ready,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ImportJob {
    spec: ImportSpec,
    identity: [u8; 32],
    digest: [u8; 32],
    cells: Vec<SourceCell>,
    walkable_tiles: u32,
    progress: ImportProgress,
}
impl ImportJob {
    pub fn new(spec: ImportSpec) -> Result<Self> {
        spec.validate()?;
        Ok(Self {
            identity: spec.identity(),
            digest: initial_digest(spec.dimensions),
            cells: Vec::with_capacity(spec.dimensions.area()),
            spec,
            walkable_tiles: 0,
            progress: ImportProgress::Loading,
        })
    }
    pub fn spec(&self) -> &ImportSpec {
        &self.spec
    }
    pub const fn progress(&self) -> ImportProgress {
        self.progress
    }
    pub fn advance(
        &mut self,
        source: &impl CellSource,
        resources: &impl Resources,
        work: u32,
    ) -> Result<ImportProgress> {
        if !(1..=1024).contains(&work) {
            return Err(Error::Bounds("import work must be 1..=1024"));
        }
        for _ in 0..work {
            if self.progress != ImportProgress::Loading {
                break;
            }
            let cell = source.cell(self.cells.len())?;
            cell.validate()?;
            let mut walkable = 0;
            for row in 0..8 {
                for column in 0..16 {
                    let overlay = if cell.cell.overlay() == 0 {
                        0
                    } else {
                        resources.overlay_tile(cell.cell.overlay(), row, column)?
                    };
                    let tile = if overlay == 0 {
                        resources.base_tile(cell.cell.base(), row, column)?
                    } else {
                        overlay
                    };
                    if matches!(resources.property(tile)?, 0x10 | 0x20) {
                        walkable += 1;
                    }
                }
            }
            self.digest = append_digest(self.digest, cell);
            self.cells.push(cell);
            self.walkable_tiles += walkable;
            if self.cells.len() == self.spec.dimensions.area() {
                self.progress =
                    if self.digest == self.spec.source_digest && self.walkable_tiles != 0 {
                        ImportProgress::Ready
                    } else {
                        ImportProgress::Rejected
                    };
            }
        }
        Ok(self.progress)
    }
    pub fn validate(&self) -> Result<()> {
        self.spec.validate()?;
        if self.identity != self.spec.identity()
            || self.cells.len() > self.spec.dimensions.area()
            || self.walkable_tiles > self.cells.len() as u32 * 128
            || (self.progress == ImportProgress::Loading)
                != (self.cells.len() < self.spec.dimensions.area())
        {
            return Err(Error::State("import continuation extent/identity/phase"));
        }
        let mut digest = initial_digest(self.spec.dimensions);
        for cell in &self.cells {
            cell.validate()?;
            digest = append_digest(digest, *cell);
        }
        if digest != self.digest
            || (self.progress == ImportProgress::Ready
                && (self.digest != self.spec.source_digest || self.walkable_tiles == 0))
            || (self.progress == ImportProgress::Rejected
                && self.digest == self.spec.source_digest
                && self.walkable_tiles != 0)
        {
            return Err(Error::State("import continuation content digest"));
        }
        Ok(())
    }
    pub fn finish(self) -> Result<(FloorSpec, GeneratedLevel, crate::land::LandState)> {
        self.validate()?;
        if self.progress != ImportProgress::Ready {
            return Err(Error::State("unready/rejected import cannot publish"));
        }
        let dimensions = self.spec.dimensions;
        let provenance = self.spec.provenance();
        let mut land = crate::land::LandState::new(dimensions)?;
        let mut cells = Vec::with_capacity(dimensions.area());
        let mut paths = Vec::with_capacity(dimensions.area());
        for (index, source) in self.cells.into_iter().enumerate() {
            cells.push(source.cell);
            paths.push(source.directions);
            if source.marked {
                land.mark(
                    dimensions.position(index).expect("bounded import"),
                    source.alternate,
                )?;
            }
        }
        let geometry = LevelGeometry::new(dimensions, cells, paths, self.spec.anchors)?;
        let generated = GeneratedLevel {
            identity: self.identity,
            accepted_attempt: 0,
            geometry,
            population: self.spec.population,
            runtime_rng: StageRng::derive(&self.identity, self.spec.level, 0, b"runtime-ai"),
            walkable_tiles: self.walkable_tiles,
        };
        Ok((FloorSpec::Imported(provenance), generated, land))
    }
}
