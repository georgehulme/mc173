//! Item use in the world.

use glam::{IVec3, DVec3, Vec3};

use crate::block::sapling::TreeKind;
use crate::gen::tree::TreeGenerator;
use crate::item::{ItemStack, self};
use crate::util::Face;
use crate::block;

use super::World;


impl World {

    /// Use an item stack on a given block, this is basically the action of left click. 
    /// This function returns the item stack after, if used, this may return an item stack
    /// with size of 0. The face is where the click has hit on the target block.
    pub fn use_stack(&mut self, stack: ItemStack, pos: IVec3, face: Face, entity_id: u32) -> Option<ItemStack> {

        if stack.is_empty() {
            return None;
        }
        
        let success = match stack.id {
            0 => false,
            1..=255 => self.use_block_stack(stack.id as u8, stack.damage as u8, pos, face, entity_id),
            item::SUGAR_CANES => self.use_block_stack(block::SUGAR_CANES, 0, pos, face, entity_id),
            item::CAKE => self.use_block_stack(block::CAKE, 0, pos, face, entity_id),
            item::REPEATER => self.use_block_stack(block::REPEATER, 0, pos, face, entity_id),
            item::REDSTONE => self.use_block_stack(block::REDSTONE, 0, pos, face, entity_id),
            item::WOOD_DOOR => self.use_door_stack(block::WOOD_DOOR, pos, face, entity_id),
            item::IRON_DOOR => self.use_door_stack(block::IRON_DOOR, pos, face, entity_id),
            item::BED => self.use_bed_stack(pos, face, entity_id),
            item::DIAMOND_HOE |
            item::IRON_HOE |
            item::STONE_HOE |
            item::GOLD_HOE |
            item::WOOD_HOE => return self.use_hoe_stack(stack, pos, face),
            item::WHEAT_SEEDS => self.use_wheat_seeds_stack(pos, face),
            item::DYE if stack.damage == 15 => self.use_bone_meal_stack(pos),
            _ => false
        };

        success.then_some(stack.with_size(stack.size - 1))

    }

    /// Use an item that is not meant to be used on blocks. Such as buckets, boats, bows or
    /// food items...
    pub fn use_raw_stack(&mut self, stack: ItemStack, entity_id: u32) -> Option<ItemStack> {

        match stack.id {
            item::BUCKET => self.use_bucket_stack(block::AIR, entity_id),
            item::WATER_BUCKET => self.use_bucket_stack(block::WATER_MOVING, entity_id),
            item::LAVA_BUCKET => self.use_bucket_stack(block::LAVA_MOVING, entity_id),
            _ => None
        }

    }

    /// Place a block toward the given face. This is used for single blocks, multi blocks
    /// are handled apart by other functions that do not rely on the block placing logic.
    fn use_block_stack(&mut self, id: u8, metadata: u8, mut pos: IVec3, mut face: Face, entity_id: u32) -> bool {

        let look = self.get_entity(entity_id).unwrap().base().look;

        if let Some((block::SNOW, _)) = self.get_block(pos) {
            // If a block is placed by clicking on a snow block, replace that snow block.
            face = Face::NegY;
        } else {
            // Get position of the block facing the clicked face.
            pos += face.delta();
            // The block is oriented toward that clicked face.
            face = face.opposite();
        }

        // Some block have special facing when placed.
        match id {
            block::WOOD_STAIR | block::COBBLESTONE_STAIR |
            block::REPEATER | block::REPEATER_LIT => {
                face = Face::from_yaw(look.x);
            }
            block::DISPENSER |
            block::FURNACE | block::FURNACE_LIT |
            block::PUMPKIN | block::PUMPKIN_LIT => {
                face = Face::from_yaw(look.x).opposite();
            }
            block::PISTON => {
                face = Face::from_look(look.x, look.y).opposite();
            }
            _ => {}
        }

        if pos.y >= 127 && block::from_id(id).material.is_solid() {
            return false;
        } if !self.can_place_block(pos, face, id) {
            return false;
        }

        self.place_block(pos, face, id, metadata);
        true

    }

    /// Place a door item at given position.
    fn use_door_stack(&mut self, block_id: u8, mut pos: IVec3, face: Face, entity_id: u32) -> bool {

        if face != Face::PosY {
            return false;
        } else {
            pos += IVec3::Y;
        }

        if pos.y >= 127 {
            return false;
        } else if !self.can_place_block(pos, face.opposite(), block_id) {
            return false;
        }

        // The door face the opposite of the placer's look.
        let look = self.get_entity(entity_id).unwrap().base().look;
        let mut door_face = Face::from_yaw(look.x).opposite();
        let mut flip = false;
        
        // Here we count the block on the left and right (from the door face), this will
        // change the default orientation of the door.
        let left_pos = pos + door_face.rotate_left().delta();
        let right_pos = pos + door_face.rotate_right().delta();

        // Temporary closure to avoid boiler plate just after.
        let is_door_block = |pos| {
            self.get_block(pos).map(|(id, _)| id == block_id).unwrap_or(false)
        };

        let left_door = is_door_block(left_pos) || is_door_block(left_pos + IVec3::Y);
        let right_door = is_door_block(right_pos) || is_door_block(right_pos + IVec3::Y);

        if right_door && !left_door {
            flip = true;
        } else {

            let left_count = 
                self.is_block_opaque_cube(left_pos) as u8 + 
                self.is_block_opaque_cube(left_pos + IVec3::Y) as u8;
            
            let right_count = 
                self.is_block_opaque_cube(right_pos) as u8 + 
                self.is_block_opaque_cube(right_pos + IVec3::Y) as u8;

            if left_count > right_count {
                flip = true;
            }

        }

        let mut metadata = 0;

        // To flip the door, we rotate it left and open it by default.
        if flip {
            block::door::set_open(&mut metadata, true);
            door_face = door_face.rotate_left();
        }

        block::door::set_face(&mut metadata, door_face);
        self.set_block_notify(pos, block_id, metadata);

        block::door::set_upper(&mut metadata, true);
        self.set_block_notify(pos + IVec3::Y, block_id, metadata);

        true

    }

    fn use_bed_stack(&mut self, mut pos: IVec3, face: Face, entity_id: u32) -> bool {

        if face != Face::PosY {
            return false;
        } else {
            pos += IVec3::Y;
        }

        let look = self.get_entity(entity_id).unwrap().base().look;
        let bed_face = Face::from_yaw(look.x);
        let head_pos = pos + bed_face.delta();

        if !matches!(self.get_block(pos), Some((block::AIR, _))) {
            return false;
        } else if !matches!(self.get_block(head_pos), Some((block::AIR, _))) {
            return false;
        } else if !self.is_block_opaque_cube(pos - IVec3::Y) || !self.is_block_opaque_cube(head_pos - IVec3::Y) {
            return false;
        }

        let mut metadata = 0;
        block::bed::set_face(&mut metadata, bed_face);
        self.set_block_notify(pos, block::BED, metadata);
        block::bed::set_head(&mut metadata, true);
        self.set_block_notify(head_pos, block::BED, metadata);

        true

    }

    fn use_hoe_stack(&mut self, stack: ItemStack, pos: IVec3, face: Face) -> Option<ItemStack> {
        
        let (id, _) = self.get_block(pos)?;
        let (above_id, _) = self.get_block(pos + IVec3::Y)?;

        if (face == Face::NegY || above_id != block::AIR || id != block::GRASS) && id != block::DIRT {
            None
        } else {
            self.set_block_notify(pos, block::FARMLAND, 0);
            Some(stack.inc_damage(1))
        }

    }

    fn use_wheat_seeds_stack(&mut self, pos: IVec3, face: Face) -> bool {

        if face == Face::PosY {
            if let Some((block::FARMLAND, _)) = self.get_block(pos) {
                if let Some((block::AIR, _)) = self.get_block(pos + IVec3::Y) {
                    self.set_block_notify(pos + IVec3::Y, block::WHEAT, 0);
                    return true;
                }
            }
        }

        false

    }

    fn use_bone_meal_stack(&mut self, pos: IVec3) -> bool {

        let Some((id, metadata)) = self.get_block(pos) else { return false };

        if id == block::SAPLING {
            
            let mut gen = match block::sapling::get_kind(metadata) {
                TreeKind::Oak if self.get_rand_mut().next_int_bounded(10) == 0 => TreeGenerator::new_big(),
                TreeKind::Oak => TreeGenerator::new_oak(),
                TreeKind::Birch => TreeGenerator::new_birch(),
                TreeKind::Spruce => TreeGenerator::new_spruce2(),
            };
            
            gen.generate_from_sapling(self, pos);
            true

        } else {
            false
        }

    }

    fn use_bucket_stack(&mut self, fluid_id: u8, entity_id: u32) -> Option<ItemStack> {

        let entity_base = self.get_entity(entity_id).unwrap().base();
        
        let origin = entity_base.pos + DVec3::new(0.0, 1.62, 0.0);
        
        let yaw_dx = -entity_base.look.x.sin();
        let yaw_dz = entity_base.look.x.cos();
        let pitch_dy = -entity_base.look.y.sin();
        let pitch_h = entity_base.look.y.cos();
        let ray = Vec3::new(yaw_dx * pitch_h, pitch_dy, yaw_dz * pitch_h).as_dvec3();

        let (pos, face) = self.ray_trace_blocks(origin, ray * 5.0, fluid_id == block::AIR)?;
        let (id, metadata) = self.get_block(pos)?;

        // The bucket is empty.
        if fluid_id == block::AIR {

            // Fluid must be a source.
            if !block::fluid::is_source(metadata) {
                return None;
            }

            let item = match id {
                block::WATER_MOVING | block::WATER_STILL => item::WATER_BUCKET,
                block::LAVA_MOVING | block::LAVA_STILL => item::LAVA_BUCKET,
                _ => return None
            };

            self.set_block_notify(pos, block::AIR, 0);

            Some(ItemStack::new_single(item, 0))

        } else {

            let pos = pos + face.delta();
            let (id, _) = self.get_block(pos)?;

            if id == block::AIR || !block::from_id(id).material.is_solid() {
                self.set_block_notify(pos, fluid_id, 0);
                // world.schedule_tick(pos, fluid_id, 5); // TODO: 30 for lava.
            }

            Some(ItemStack::new_single(item::BUCKET, 0))

        }

    }


}