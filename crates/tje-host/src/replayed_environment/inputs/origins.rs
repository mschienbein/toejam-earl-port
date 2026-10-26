//! Per-bit causal inputs. Read identities are actual occurrences, not addresses
//! or samples; each occurrence binds once at its physical chronology marker.
//! Completed output providers own this graph, never an arena reused by a call.
use super::{
    Access, Direction, Failure, StateField, StateMask, canonical_ram_address, unsupported,
};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Origin(usize);
impl Origin {
    pub const EMPTY: Self = Self(0);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::replayed_environment) struct MemoryInput {
    pub value: u8,
    pub bits: u8,
}
#[derive(Debug, Default)]
pub(in crate::replayed_environment) struct Demand {
    pub registers: StateMask,
    pub memory: BTreeMap<u32, MemoryInput>,
}
#[derive(Debug, Clone, Copy)]
enum Node {
    Empty,
    Register { field: StateField, bits: u32 },
    InitialRam { address: u32, value: u8, bits: u8 },
    Union(Origin, Origin),
    Read { occurrence: usize, bits: u16 },
    StackTransport { source: Origin, bank: u8 },
}
struct Read {
    access: Access,
    driven: u16,
    bound: Option<[Origin; 16]>,
}
#[derive(Default)]
struct Query {
    marks: Vec<[u8; 4]>,
    touched: Vec<(usize, u8)>,
    work: Vec<(Origin, bool, u8)>,
}
struct Graph {
    nodes: Vec<Node>,
    unions: BTreeMap<(Origin, Origin), Origin>,
    read_nodes: BTreeMap<(usize, u16), Origin>,
    reads: Vec<Read>,
    query: RefCell<Query>,
}
impl Graph {
    fn new() -> Self {
        let mut nodes = Vec::with_capacity(StateField::COUNT * 32 + 1);
        nodes.push(Node::Empty);
        Self {
            nodes,
            unions: BTreeMap::new(),
            read_nodes: BTreeMap::new(),
            reads: Vec::new(),
            query: RefCell::new(Query::default()),
        }
    }
    fn add(&mut self, node: Node) -> Origin {
        let origin = Origin(self.nodes.len());
        self.nodes.push(node);
        origin
    }
    fn union(&mut self, a: Origin, b: Origin) -> Origin {
        if a == Origin::EMPTY || a == b {
            return b;
        }
        if b == Origin::EMPTY {
            return a;
        }
        let pair = if a < b { (a, b) } else { (b, a) };
        if let Some(&origin) = self.unions.get(&pair) {
            return origin;
        }
        let origin = self.add(Node::Union(pair.0, pair.1));
        self.unions.insert(pair, origin);
        origin
    }
    fn demand(&self, root: Origin) -> Result<Demand, Failure> {
        self.demand_many(std::iter::once(root))
    }
    fn demand_many(&self, roots: impl IntoIterator<Item = Origin>) -> Result<Demand, Failure> {
        let mut result = Demand::default();
        let mut query = self.query.borrow_mut();
        let Query {
            marks,
            touched,
            work,
        } = &mut *query;
        for (index, bank) in touched.drain(..) {
            marks[index][usize::from(bank)] = 0;
        }
        marks.resize(self.nodes.len(), [0; 4]);
        work.clear();
        work.extend(roots.into_iter().map(|root| (root, false, 0)));
        while let Some((origin, exit, transport)) = work.pop() {
            let Some(&node) = self.nodes.get(origin.0) else {
                return Err(unsupported(
                    "causal input refers outside its immutable graph",
                ));
            };
            let mark = &mut marks[origin.0][usize::from(transport)];
            if exit {
                *mark = 2;
                continue;
            }
            match *mark {
                1 => return Err(unsupported("cyclic physical-read input provenance")),
                2 => continue,
                _ => {}
            }
            *mark = 1;
            touched.push((origin.0, transport));
            work.push((origin, true, transport));
            match node {
                Node::Empty => {}
                Node::Register { field, bits } => {
                    let bank = match field {
                        StateField::Usp => 1,
                        StateField::Ssp => 2,
                        _ => 0,
                    };
                    if bank == 0 || transport & bank == 0 {
                        result.registers.insert(field, bits);
                    }
                }
                Node::InitialRam {
                    address,
                    value,
                    bits,
                } => {
                    let input = result
                        .memory
                        .entry(address)
                        .or_insert(MemoryInput { value, bits: 0 });
                    if (input.value ^ value) & input.bits & bits != 0 {
                        return Err(unsupported(
                            "initial RAM labels disagree on demanded input bits",
                        ));
                    }
                    input.value = (input.value & !bits) | (value & bits);
                    input.bits |= bits;
                }
                Node::Union(a, b) => {
                    work.push((b, false, transport));
                    work.push((a, false, transport));
                }
                Node::StackTransport { source, bank } => {
                    work.push((source, false, transport | bank));
                }
                Node::Read { occurrence, bits } => {
                    let read = self
                        .reads
                        .get(occurrence)
                        .ok_or_else(|| unsupported("causal read occurrence is out of range"))?;
                    let sources = read.bound.as_ref().ok_or_else(|| {
                        unsupported("causal read occurrence has no physical chronology marker")
                    })?;
                    let width = if read.access.word { u16::MAX } else { 0xff };
                    if bits & !width != 0 || bits & !read.driven != 0 {
                        return Err(unsupported(format!(
                            "actual read at {:06x} has undriven demanded bits {:04x}",
                            read.access.address,
                            bits & !read.driven
                        )));
                    }
                    for (bit, &source) in sources.iter().enumerate() {
                        if bits & (1 << bit) != 0 {
                            work.push((source, false, transport));
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

pub(super) struct Origins {
    graph: Graph,
    pub fields: Box<[[Origin; 32]; StateField::COUNT]>,
    ram: BTreeMap<u32, [Origin; 8]>,
    values: BTreeMap<u32, u8>,
}
impl Origins {
    pub fn new() -> Self {
        let mut graph = Graph::new();
        let mut fields = Box::new([[Origin::EMPTY; 32]; StateField::COUNT]);
        for (field, bits) in StateMask::ALL.registers_only().iter() {
            for bit in 0..32 {
                let mask = 1 << bit;
                if bits & mask != 0 {
                    fields[field.index() as usize][bit] =
                        graph.add(Node::Register { field, bits: mask });
                }
            }
        }
        Self {
            graph,
            fields,
            ram: BTreeMap::new(),
            values: BTreeMap::new(),
        }
    }
    pub fn union(&mut self, a: Origin, b: Origin) -> Origin {
        self.graph.union(a, b)
    }
    pub fn stack_transport(
        &mut self,
        source: Origin,
        field: StateField,
    ) -> Result<Origin, Failure> {
        let bank = match field {
            StateField::Usp => 1,
            StateField::Ssp => 2,
            _ => {
                return Err(unsupported(
                    "private-base transport names no physical stack bank",
                ));
            }
        };
        Ok(if source == Origin::EMPTY {
            source
        } else {
            self.graph.add(Node::StackTransport { source, bank })
        })
    }
    pub fn registers(&mut self, mask: StateMask) -> Origin {
        let mut result = Origin::EMPTY;
        for (field, mut bits) in mask.registers_only().iter() {
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                result = self
                    .graph
                    .union(result, self.fields[field.index() as usize][bit]);
                bits &= bits - 1;
            }
        }
        result
    }
    pub fn clear_bits(&mut self, field: StateField, mut bits: u32) {
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            self.fields[field.index() as usize][bit] = Origin::EMPTY;
            bits &= bits - 1;
        }
    }
    pub fn new_read(&mut self, access: Access, driven: u16) -> Result<usize, Failure> {
        if !matches!(access.direction, Direction::Read(_)) {
            return Err(unsupported(
                "write issue cannot mint a causal read occurrence",
            ));
        }
        let occurrence = self.graph.reads.len();
        // The existing chronology journal contains non-ROM reads only. Below
        // its explicit threshold there cannot be a work-RAM dependency; raw
        // issue/context binding and driven-mask checks remain independent.
        let bound = (access.address < 0x800000).then_some([Origin::EMPTY; 16]);
        self.graph.reads.push(Read {
            access,
            driven,
            bound,
        });
        Ok(occurrence)
    }
    pub fn read_source(&mut self, occurrence: usize, bits: u16) -> Result<Origin, Failure> {
        let read = self
            .graph
            .reads
            .get(occurrence)
            .ok_or_else(|| unsupported("input query selected a missing physical read"))?;
        let width = if read.access.word { u16::MAX } else { 0xff };
        if bits & !width != 0 {
            return Err(unsupported(
                "input query exceeds its actual physical read width",
            ));
        }
        if bits == 0 {
            return Ok(Origin::EMPTY);
        }
        if let Some(&source) = self.graph.read_nodes.get(&(occurrence, bits)) {
            return Ok(source);
        }
        let source = self.graph.add(Node::Read { occurrence, bits });
        self.graph.read_nodes.insert((occurrence, bits), source);
        Ok(source)
    }
    fn ram_read(&mut self, address: u32, value: u8) -> Result<[Origin; 8], Failure> {
        let address = canonical_ram_address(address);
        if self
            .values
            .get(&address)
            .is_some_and(|&before| before != value)
        {
            return Err(unsupported(
                "RAM input changed without an authenticated preceding physical write",
            ));
        }
        self.values.insert(address, value);
        if let Some(&sources) = self.ram.get(&address) {
            return Ok(sources);
        }
        let sources = std::array::from_fn(|bit| {
            self.graph.add(Node::InitialRam {
                address,
                value,
                bits: 1 << bit,
            })
        });
        self.ram.insert(address, sources);
        Ok(sources)
    }
    pub fn bind_read(&mut self, occurrence: usize, access: Access) -> Result<(), Failure> {
        let read = self.graph.reads.get(occurrence).ok_or_else(|| {
            unsupported("physical read marker refers outside its read occurrences")
        })?;
        if read.access != access || read.bound.is_some() {
            return Err(unsupported(
                "physical read marker is changed, duplicated or already bound",
            ));
        }
        let mut sources = [Origin::EMPTY; 16];
        if access.ram() {
            for (lane, (address, value)) in access.bytes().enumerate() {
                let start = if access.word && lane == 0 { 8 } else { 0 };
                sources[start..start + 8].copy_from_slice(&self.ram_read(address, value)?);
            }
        }
        self.graph.reads[occurrence].bound = Some(sources);
        Ok(())
    }
    pub fn check_read(&self, occurrence: usize) -> Result<(), Failure> {
        if self
            .graph
            .reads
            .get(occurrence)
            .is_none_or(|read| read.bound.is_none())
        {
            return Err(unsupported(
                "actual physical read lacks its exact chronology marker",
            ));
        }
        Ok(())
    }
    pub fn store(&mut self, access: Access, sources: &[Origin; 16]) {
        if !access.ram() {
            return;
        }
        for (lane, (address, value)) in access.bytes().enumerate() {
            let address = canonical_ram_address(address);
            let start = if access.word && lane == 0 { 8 } else { 0 };
            let mut byte = [Origin::EMPTY; 8];
            byte.copy_from_slice(&sources[start..start + 8]);
            self.ram.insert(address, byte);
            self.values.insert(address, value);
        }
    }
    pub fn dma(&mut self, access: Access) -> Result<Origin, Failure> {
        let mut source = Origin::EMPTY;
        if access.ram() {
            for (address, value) in access.bytes() {
                for bit in self.ram_read(address, value)? {
                    source = self.graph.union(source, bit);
                }
            }
        }
        Ok(source)
    }
    pub fn demand(&self, origin: Origin) -> Result<Demand, Failure> {
        self.graph.demand(origin)
    }
    pub fn finish(self) -> OutputInputs {
        OutputInputs {
            graph: self.graph,
            fields: self.fields,
        }
    }
}

/// A completed call owns every source node and physical-read binding referenced
/// by its endpoint fields. No subsequent call/ISR can clear or repair this data.
pub(in crate::replayed_environment) struct OutputInputs {
    graph: Graph,
    fields: Box<[[Origin; 32]; StateField::COUNT]>,
}
impl OutputInputs {
    pub fn demand(&self, field: StateField, mut bits: u32) -> Result<Demand, Failure> {
        if matches!(field, StateField::Data(index) if index >= 8)
            || matches!(field, StateField::Address(index) if index >= 7)
        {
            return Err(unsupported("endpoint input field is not canonical"));
        }
        let index = field.index() as usize;
        let values = self
            .fields
            .get(index)
            .ok_or_else(|| unsupported("endpoint input field is not canonical"))?;
        if StateField::from_index(index) != Some(field) {
            return Err(unsupported(
                "endpoint input field aliases another canonical domain",
            ));
        }
        self.graph.demand_many(std::iter::from_fn(|| {
            if bits == 0 {
                return None;
            }
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            Some(values[bit])
        }))
    }
    pub fn memory_addresses(&self) -> impl Iterator<Item = u32> + '_ {
        self.graph.nodes.iter().filter_map(|node| match *node {
            Node::InitialRam { address, .. } => Some(address),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests;
