//! Block drop behaviors.

use std::ops::Add;
use std::ops::Mul;

use glam::IVec3;

use crate::entity::ItemEntity;
use crate::util::JavaRandom;
use crate::item::ItemStack;
use crate::entity::Entity;
use crate::world::World;
use crate::block;
use crate::item;


/// Drop the given block/metadata items at the given position. 
pub fn drop_at(world: &mut World, pos: IVec3, id: u8, metadata: u8, chance: f32) {
    let tries = drop_tries(world.rand_mut(), id, metadata);
    for _ in 0..tries {
        if world.rand_mut().next_float() <= chance {
            let stack = drop_stack(world.rand_mut(), id, metadata);
            if !stack.is_empty() {
                drop_stack_at(world, pos, stack, 0.7);
            }
        }
    }
}


/// Drop an item stack at the given block position.
pub fn drop_stack_at(world: &mut World, pos: IVec3, stack: ItemStack, spread: f32) {
    
    let delta = world.rand_mut().next_vec3()
        .mul(spread)
        .as_dvec3()
        .add((1.0 - spread as f64) * 0.5);

    let mut entity = ItemEntity::default();
    entity.pos = pos.as_dvec3() + delta;
    entity.vel.x = world.rand_mut().next_double() * 0.2 - 0.1;
    entity.vel.y = 0.2;
    entity.vel.z = world.rand_mut().next_double() * 0.2 - 0.1;
    entity.kind.stack = stack;
    entity.kind.frozen_ticks = 10;

    world.spawn_entity(Entity::Item(entity));

}

/// Get the tries count from a block and metadata.
pub fn drop_tries(rand: &mut JavaRandom, id: u8, _metadata: u8) -> u8 {
    match id {
        block::AIR => 0,
        block::BOOKSHELF => 0,
        block::CAKE => 0,
        block::CLAY => 4,
        block::WHEAT => 1,
        block::FIRE => 0,
        block::WATER_MOVING |
        block::WATER_STILL |
        block::LAVA_MOVING |
        block::LAVA_STILL => 0,
        block::GLASS => 0,
        block::GLOWSTONE => 2 + rand.next_int_bounded(3) as u8,
        block::ICE => 0,
        block::LEAVES if rand.next_int_bounded(20) != 0 => 0,
        block::SPAWNER => 0,
        block::LAPIS_ORE => 4 + rand.next_int_bounded(5) as u8,
        block::PISTON_EXT |
        block::PISTON_MOVING => 0,
        block::PORTAL => 0,
        block::REDSTONE_ORE |
        block::REDSTONE_ORE_LIT => 4 + rand.next_int_bounded(2) as u8,
        block::SNOW => 0,
        block::SNOW_BLOCK => 4,
        block::DOUBLE_SLAB => 2,
        block::TNT => 0,
        _ => 1
    }
}

/// Get the drop item stack from a block and metadata. This is called for each try.
pub fn drop_stack(rand: &mut JavaRandom, id: u8, metadata: u8) -> ItemStack {
    match id {
        // Bed only drop if not head piece. 
        block::BED if block::bed::is_head(metadata) => ItemStack::EMPTY,
        block::BED => ItemStack::new_single(item::BED, 0),
        // Cake.
        block::CAKE => ItemStack::EMPTY,
        // Clay.
        block::CLAY => ItemStack::new_single(item::CLAY, 0),
        // Wheat, only drop if reached max stage.
        block::WHEAT if metadata != 7 => return ItemStack::EMPTY,
        block::WHEAT => ItemStack::new_single(item::WHEAT, 0),
        // Dead bush.
        block::DEAD_BUSH => ItemStack::EMPTY,
        // Door only drop if lower part.
        block::WOOD_DOOR | 
        block::IRON_DOOR if block::door::is_upper(metadata) => ItemStack::EMPTY,
        block::WOOD_DOOR => ItemStack::new_single(item::WOOD_DOOR, 0),
        block::IRON_DOOR => ItemStack::new_single(item::IRON_DOOR, 0),
        // Farmland and grass.
        block::FARMLAND |
        block::GRASS => ItemStack::new_block(block::DIRT, 0),
        // Fluids.
        block::WATER_MOVING |
        block::WATER_STILL |
        block::LAVA_MOVING |
        block::LAVA_STILL => ItemStack::EMPTY,
        // Furnace.
        block::FURNACE |
        block::FURNACE_LIT => ItemStack::new_block(block::FURNACE, 0),
        // Glowstone.
        block::GLOWSTONE => ItemStack::new_single(item::GLOWSTONE_DUST, 0),
        // Gravel.
        block::GRAVEL if rand.next_int_bounded(10) == 0 => ItemStack::new_single(item::FLINT, 0),
        // Leaves.
        block::LEAVES => ItemStack::new_block(block::SAPLING, metadata & 3),
        // Spawner.
        block::SPAWNER => ItemStack::EMPTY,
        // Ores.
        block::COAL_ORE => ItemStack::new_single(item::COAL, 0),
        block::DIAMOND_ORE => ItemStack::new_single(item::DIAMOND, 0),
        block::REDSTONE_ORE |
        block::REDSTONE_ORE_LIT => ItemStack::new_single(item::REDSTONE, 0),
        block::LAPIS_ORE => ItemStack::new_single(item::DYE, 4),
        // Piston.
        block::PISTON_EXT |
        block::PISTON_MOVING => ItemStack::EMPTY,
        // Redstone components.
        block::REDSTONE => ItemStack::new_single(item::REDSTONE, 0),
        block::REPEATER |
        block::REPEATER_LIT => ItemStack::new_single(item::REPEATER, 0),
        block::REDSTONE_TORCH |
        block::REDSTONE_TORCH_LIT => ItemStack::new_block(block::REDSTONE_TORCH_LIT, 0),
        // Sugar cane.
        block::SUGAR_CANES => ItemStack::new_single(item::SUGAR_CANES, 0),
        // Signs.
        block::SIGN |
        block::WALL_SIGN => ItemStack::new_single(item::SIGN, 0),
        // Snow.
        block::SNOW_BLOCK |
        block::SNOW => ItemStack::new_single(item::SNOWBALL, 0),
        // Double slab.
        block::SLAB |
        block::DOUBLE_SLAB => ItemStack::new_block(block::SLAB, metadata),
        // Stone.
        block::STONE => ItemStack::new_block(block::COBBLESTONE, 0),
        // Tall grass.
        block::TALL_GRASS if rand.next_int_bounded(8) == 0 => ItemStack::new_single(item::WHEAT_SEEDS, 0),
        block::TALL_GRASS => ItemStack::EMPTY,
        // Cobweb.
        block::COBWEB => ItemStack::new_single(item::STRING, 0),
        // Log type.
        block::LOG => ItemStack::new_block(block::LOG, metadata),
        // Wool color.
        block::WOOL => ItemStack::new_block(block::WOOL, metadata),
        // Sapling type.
        block::SAPLING => ItemStack::new_block(block::SAPLING, metadata & 3),
        // Default, drop the block's item.
        _ => ItemStack::new_block(id, 0),
    }
}
