//! Data structure for storing a world (overworld or nether) at runtime.

use std::collections::HashMap;
use std::iter::FusedIterator;
use std::ops::Add;

use glam::{IVec3, DVec3};

use crate::chunk::{Chunk, calc_chunk_pos};
use crate::util::bb::BoundingBox;
use crate::block::block_from_id;
use crate::entity::Entity;


/// Calculate the chunk position corresponding to the given block position. 
/// This also returns chunk-local coordinates in this chunk.
#[inline]
pub fn calc_entity_chunk_pos(pos: DVec3) -> (i32, i32) {
    calc_chunk_pos(pos.as_ivec3())
}


/// Data structure for a whole world.
pub struct World {
    /// The dimension
    dimension: Dimension,
    /// The spawn position.
    spawn_pos: IVec3,
    /// The world time, increasing at each tick.
    time: u64,
    /// Mapping of chunks to their coordinates.
    chunks: HashMap<(i32, i32), Box<Chunk>>,
    /// The entities are stored inside an option, this has no overhead because of niche 
    /// in the box type, but allows us to temporarily own the entity when updating it, 
    /// therefore avoiding borrowing issues.
    entities: Vec<Option<Box<dyn Entity>>>,
    /// Entities' index mapping from their unique id.
    entities_map: HashMap<u32, usize>,
}

impl World {

    pub fn new(dimension: Dimension) -> Self {
        Self {
            dimension,
            spawn_pos: IVec3::ZERO,
            time: 0,
            chunks: HashMap::new(),
            entities: Vec::new(),
            entities_map: HashMap::new(),
        }
    }

    pub fn dimension(&self) -> Dimension {
        self.dimension
    }

    pub fn spawn_pos(&self) -> IVec3 {
        self.spawn_pos
    }

    pub fn set_spawn_pos(&mut self, pos: IVec3) {
        self.spawn_pos = pos;
    }

    pub fn time(&self) -> u64 {
        self.time
    }

    pub fn set_time(&mut self, time: u64) {
        self.time = time;
    }

    pub fn chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> {
        self.chunks.get(&(cx, cz)).map(|c| &**c)
    }

    pub fn chunk_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Chunk> {
        self.chunks.get_mut(&(cx, cz)).map(|c| &mut **c)
    }

    pub fn insert_chunk(&mut self, cx: i32, cz: i32, chunk: Box<Chunk>) {
        self.chunks.insert((cx, cz), chunk);
    }

    pub fn remove_chunk(&mut self, cx: i32, cz: i32) -> Option<Box<Chunk>> {
        self.chunks.remove(&(cx, cz))
    }

    /// Spawn an entity in this world.
    pub fn spawn_entity(&mut self, entity: Box<dyn Entity>) -> u32 {

        let id = entity.base().id;
        let index = self.entities.len();

        self.entities.push(Some(entity));
        self.entities_map.insert(id, index);

        id

    }

    /// Tick the world, this ticks all entities.
    pub fn tick(&mut self) {

        // For each entity, we take the box from its slot (moving 64 * 2 bits), therefore
        // taking the ownership, before ticking it with the mutable world.
        for i in 0..self.entities.len() {
            
            // We unwrap because all entities should be present except updated one.
            let mut entity = self.entities[i].take().unwrap();
            entity.tick(&mut *self);
            // After tick, we re-add the entity.
            self.entities[i] = Some(entity);

        }

    }
    
    /// Iterate over all blocks in the given area.
    /// Min is inclusive and max is exclusive.
    #[must_use]
    pub fn iter_area_blocks(&self, min: IVec3, max: IVec3) -> impl Iterator<Item = (u8, u8)> + FusedIterator + '_ {
        WorldAreaBlocks {
            world: self,
            chunk: None,
            min,
            max,
            cursor: min,
        }
    }

    /// Iterate over all bounding boxes in the given area.
    /// Min is inclusive and max is exclusive.
    #[must_use]
    pub fn iter_area_bounding_boxes(&self, min: IVec3, max: IVec3) -> impl Iterator<Item = BoundingBox> + '_ {
        self.iter_area_blocks(min, max).flat_map(|(id, metadata)| block_from_id(id).bounding_boxes(metadata).iter().copied())
    }

    #[must_use]
    pub fn iter_colliding_bounding_boxes(&self, bb: BoundingBox) -> impl Iterator<Item = BoundingBox> + '_ {
        let min = bb.min.floor().as_ivec3();
        let max = bb.max.add(1.0).floor().as_ivec3();
        self.iter_area_bounding_boxes(min, max).filter(move |block_bb| block_bb.intersects(bb))
    }

}

/// Types of dimensions, used for ambient effects in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    /// The overworld dimension with a blue sky and day cycles.
    Overworld,
    /// The creepy nether dimension.
    Nether,
}


/// An iterator for blocks in a world area. This returns the block id and metadata.
struct WorldAreaBlocks<'a> {
    /// Back-reference to the containing world.
    world: &'a World,
    /// This contains a temporary reference to the chunk being analyzed. This is used to
    /// avoid repeatedly fetching chunks' map.
    chunk: Option<(i32, i32, Option<&'a Chunk>)>,
    /// Minimum coordinate to fetch.
    min: IVec3,
    /// Maximum coordinate to fetch (exclusive).
    max: IVec3,
    /// Next block to fetch.
    cursor: IVec3,
}

impl<'a> FusedIterator for WorldAreaBlocks<'a> {}
impl<'a> Iterator for WorldAreaBlocks<'a> {

    type Item = (u8, u8);

    fn next(&mut self) -> Option<Self::Item> {
        
        let cursor = self.cursor;

        if cursor == self.max {
            return None;
        }

        // We are at the start of a new column, update the chunk.
        if cursor.y == self.min.y {
            let (cx, cz) = calc_chunk_pos(cursor);
            if !matches!(self.chunk, Some((ccx, ccz, _)) if ccx == cx && ccz == cz) {
                self.chunk = Some((cx, cz, self.world.chunk(cx, cz)));
            }
        }

        // If there is no chunk at the position, defaults to (id = 0, metadata = 0).
        let mut ret = (0, 0);

        // If a chunk exists for the current column.
        if let Some((_, _, Some(chunk))) = self.chunk {
            ret = chunk.block_and_metadata(self.cursor);
        }

        self.cursor.y += 1;
        if self.cursor.y == self.max.y {
            self.cursor.y = self.min.y;
            self.cursor.z += 1;
            if self.cursor.z == self.max.z {
                self.cursor.z = self.min.z;
                self.cursor.x += 1;
            }
        }

        Some(ret)

    }

}