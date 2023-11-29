//! World generation module.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::source::{ChunkSource, ChunkSourceError};
use crate::world::{World, ChunkSnapshot, Dimension};
use crate::chunk::Chunk;


mod cave;
mod overworld;

pub use overworld::OverworldGenerator;


const POPULATED_NEG_NEG: u8 = 0b0001;
const POPULATED_POS_NEG: u8 = 0b0010;
const POPULATED_NEG_POS: u8 = 0b0100;
const POPULATED_POS_POS: u8 = 0b1000;
const POPULATED_ALL: u8     = 0b1111;
const POPULATED_NEG_X: u8   = POPULATED_NEG_NEG | POPULATED_NEG_POS;
const POPULATED_POS_X: u8   = POPULATED_POS_POS | POPULATED_POS_NEG;
const POPULATED_NEG_Z: u8   = POPULATED_NEG_NEG | POPULATED_POS_NEG;
const POPULATED_POS_Z: u8   = POPULATED_POS_POS | POPULATED_NEG_POS;


/// Chunk source for generating a world.
pub struct GeneratorChunkSource<G: ChunkGenerator> {
    /// The inner generator immutable structure shared between all workers.
    shared: Arc<GeneratorShared<G>>,
    /// The owned cache for the generator.
    cache: G::Cache,
    /// The internal world used for chunk population and entity spawning at generation.
    /// Chunks are temporarily added to this world when not fully populated, and then
    /// removed when fully populated to be returned from source.
    world: World,
    /// For each chunk present in the world, this tells wether it is populated or not.
    populated: HashMap<(i32, i32), u8>,
}

/// Shared data between all workers.
struct GeneratorShared<G: ChunkGenerator> {
    /// The immutable generator.
    generator: G,
    /// The internal cache of chunks that only have terrain generated.
    terrain_chunks: RwLock<HashMap<(i32, i32), Arc<Chunk>>>,
}

impl<G> GeneratorChunkSource<G>
where
    G: ChunkGenerator,
    G::Cache: Default,
{

    /// Create a new generator with it default cache, this generator can then be cloned
    /// if desired and the cache will remain shared between all generators.
    #[inline]
    pub fn new(generator: G) -> Self {
        Self {
            shared: Arc::new(GeneratorShared { 
                generator, 
                terrain_chunks: RwLock::new(HashMap::new()),
            }),
            cache: Default::default(),
            // The dimension is not relevant here.
            world: World::new(Dimension::Overworld),
            populated: HashMap::new(),
        }
    }

}

// Had to manually implement Clone because derive could not figure out how to do 
// with cache being clone or not.
impl<G> Clone for GeneratorChunkSource<G>
where
    G: ChunkGenerator,
    G::Cache: Clone
{

    fn clone(&self) -> Self {
        Self { 
            shared: Arc::clone(&self.shared), 
            cache: self.cache.clone(),
            world: self.world.clone(),
            populated: self.populated.clone(),
        }
    }

}

impl<G> ChunkSource for GeneratorChunkSource<G>
where
    G: ChunkGenerator,
{

    type LoadError = ();
    type SaveError = ();

    fn load(&mut self, cx: i32, cz: i32) -> Result<ChunkSnapshot, ChunkSourceError<Self::LoadError>> {

        // The chunk may already be generated or partially populated, so we check which
        // chunks needs to be generated in order to be populated.
        let populated = self.populated.get(&(cx, cz)).copied().unwrap_or(0);
        assert_ne!(populated, POPULATED_ALL, "incoherent");

        let mut min_cx = cx;
        let mut min_cz = cz;
        let mut max_cx = cx;
        let mut max_cz = cz;
        
        // Only generate terrain for chunks on corners that are not yet populated.
        if populated & POPULATED_NEG_X != POPULATED_NEG_X {
            min_cx -= 1;
        }
        if populated & POPULATED_POS_X != POPULATED_POS_X {
            max_cx += 1;
        }
        if populated & POPULATED_NEG_Z != POPULATED_NEG_Z {
            min_cz -= 1;
        }
        if populated & POPULATED_POS_Z != POPULATED_POS_Z {
            max_cz += 1;
        }

        // For each chunk that needs to be loaded, we check if its terrain already exists,
        // if not existing then we generate it. Note that two workers may generate the
        // same chunk at the same time, but it's not a problem because only one will add
        // its chunk to the shared map, and we keep a shared reference to the generate
        // chunk to we are sure after this loop that all required chunks are present in
        // the internal world.
        for terrain_cx in min_cx..=max_cx {
            for terrain_cz in min_cz..=max_cz {

                // Do not override if we already have the chunk.
                if !self.world.contains_chunk(terrain_cx, terrain_cz) {

                    let chunks = self.shared.terrain_chunks.read().unwrap();
                    if let Some(chunk) = chunks.get(&(terrain_cx, terrain_cz)) {
                        self.world.set_chunk(terrain_cx, terrain_cz, Arc::clone(chunk));
                    } else {

                        // Allow other workers to check if a chunk exists.
                        drop(chunks);

                        let mut terrain_chunk = Chunk::new();
                        let chunk_access = Arc::get_mut(&mut terrain_chunk).unwrap();
                        self.shared.generator.generate(terrain_cx, terrain_cz, chunk_access, &mut self.cache);

                        // It's rare but two workers may generate the same chunk if slow.
                        let mut chunks = self.shared.terrain_chunks.write().unwrap();
                        let chunk = chunks.entry((terrain_cx, terrain_cz)).or_insert(terrain_chunk);

                        self.world.set_chunk(terrain_cx, terrain_cz, Arc::clone(chunk));

                    }

                    self.populated.insert((terrain_cx, terrain_cz), 0);

                }

            }
        }

        // Now that we have all our terrain chunks, we can generate the chunks. We also
        // update the populated flag of each generated chunk.
        for populate_cx in min_cx..=max_cx - 1 {
            for populate_cz in min_cz..=max_cz - 1 {
                
                self.shared.generator.populate(populate_cx, populate_cz, &mut self.world, &mut self.cache);

                // This is a bit complex to compute, maybe improve in the future.
                *self.populated.get_mut(&(populate_cx    , populate_cz    )).unwrap() |= POPULATED_POS_POS;
                *self.populated.get_mut(&(populate_cx + 1, populate_cz    )).unwrap() |= POPULATED_NEG_POS;
                *self.populated.get_mut(&(populate_cx    , populate_cz + 1)).unwrap() |= POPULATED_POS_NEG;
                *self.populated.get_mut(&(populate_cx + 1, populate_cz + 1)).unwrap() |= POPULATED_NEG_NEG;

            }
        }

        let populated = self.populated.remove(&(cx, cz)).expect("chunk should be present");
        assert_eq!(populated, POPULATED_ALL, "chunk should be fully populated at this point");

        Ok(self.world.remove_chunk_snapshot(cx, cz).expect("chunk should be present"))

    }

}


/// A trait common to all chunk generators, such generator can be used as a chunk source
/// through a [`GeneratorChunkSource`] object.
pub trait ChunkGenerator {

    /// Type of the cache that is only owned by a single worker, the generator itself
    /// should however be 
    type Cache;

    /// Generate the chunk terrain but do not populate it.
    fn generate(&self, cx: i32, cz: i32, chunk: &mut Chunk, cache: &mut Self::Cache);

    /// Populate a chunk that is present in a world, note that this world is internal
    /// to the generator, this chunk will then be transferred to the real world when
    /// done. Populate usually applies with an offset of 8 blocks into the chunk with
    /// a 16x16 populate area, this means that neighbor chunks affected are also
    /// guaranteed to be loaded.
    fn populate(&self, cx: i32, cz: i32, world: &mut World, cache: &mut Self::Cache);

}