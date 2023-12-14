//! Base function for ticking entity.
//! 
//! This module gives in documentation the reference to the Java methods, as known from
//! the decompilation of Minecraft b1.7.3 by RetroMCP.

use std::cell::RefCell;
use std::ops::Add;

use glam::{DVec3, IVec3, Vec2};

use log::{trace, log_enabled, Level};

use crate::item::ItemStack;
use crate::path::PathFinder;
use crate::util::Face;
use crate::world::{World, Event, EntityEvent};
use crate::block::{self, Material};

use super::{Entity, Size, Path,
    BaseKind, ProjectileKind, LivingKind, 
    Base, Living, 
    Item, Painting, FallingBlock};


/// This implementation is just a wrapper to call all the inner tick functions.
impl Entity {

    /// This this entity from its id in a world.
    pub fn tick(&mut self, world: &mut World, id: u32) {
        // This function just forwards to the correct tick method.
        tick_base(world, id, &mut self.0, &mut self.1);
    }

}


thread_local! {
    /// This thread local vector is currently used to list all entities that may be picked
    /// up by the entity being updated, if it have the `can_pickup` capability.
    static PICKED_UP_ENTITIES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}


/// Tick base method that is common to every entity kind.
/// 
/// REF: Entity::onUpdate
fn tick_base(world: &mut World, id: u32, base: &mut Base, base_kind: &mut BaseKind) {

    // Just kill the entity if far in the void.
    if base.pos.y < -64.0 {
        world.remove_entity(id);
        return;
    }

    // If size is not coherent, get the current size and initialize the bounding box
    // from the current position.
    if !base.coherent {
        base.size = calc_size(base_kind);
        base.update_bounding_box_from_pos();
    } else if base.controlled {
        base.update_bounding_box_from_pos();
    }

    // Increase the entity lifetime, used by some entities and is interesting for debug.
    base.lifetime += 1;

    // Do not tick entity logic if the entity is externally controlled.
    if !base.controlled {
        match base_kind {
            BaseKind::Item(item) => tick_item(world, id, base, item),
            BaseKind::Painting(painting) => tick_painting(world, id, base, painting),
            BaseKind::Boat(_) => todo!(),
            BaseKind::Minecart(_) => todo!(),
            BaseKind::Fish(_) => todo!(),
            BaseKind::LightningBolt(_) => todo!(),
            BaseKind::FallingBlock(falling_block) => tick_falling_block(world, id, base, falling_block),
            BaseKind::Tnt(_) => todo!(),
            BaseKind::Projectile(_, _) => todo!(),
            BaseKind::Living(living, living_kind) => tick_living(world, id, base, living, living_kind),
        }
    }

    tick_base_state(world, id, base, base_kind);

    // Only trace each second.
    if base.lifetime % 4 == 0 && base.pos_dirty && log_enabled!(Level::Trace) {
        let kind = base_kind.entity_kind();
        let bb_size = base.bb.size();
        trace!("entity #{id} ({kind:?}), pos: {:.2}/{:.2}/{:.2}, bb: {:.2}/{:.2}/{:.2} -> {:.2}/{:.2}/{:.2} ({:.2}/{:.2}/{:.2})", 
            base.pos.x, base.pos.y, base.pos.z, 
            base.bb.min.x, base.bb.min.y, base.bb.min.z,
            base.bb.max.x, base.bb.max.y, base.bb.max.z,
            bb_size.x, bb_size.y, bb_size.z,
        );
    }

}

/// Tick base method that is common to every entity kind, this is split in Notchian impl
/// so we split it here.
/// 
/// REF: Entity::onEntityUpdate
fn tick_base_state(world: &mut World, id: u32, base: &mut Base, base_kind: &mut BaseKind) {

    // TODO: Handle water velocity.
    base.in_water = false;

    if base.in_water {
        base.fire_time = 0;
        base.fall_distance = 0.0;
    } else if base.fire_immune {
        base.fire_time = 0;
    }

    if base.fire_time != 0 {
        if false { // if fire immune
            base.fire_time = base.fire_time.saturating_sub(4);
        } else {
            if base.fire_time % 20 == 0 {
                // TODO: Damage entity
            }
            base.fire_time -= 1;
        }
    }

    // Check if there is a lava block colliding...
    let lava_bb = base.bb.inflate(DVec3::new(-0.1, -0.4, -0.1));
    base.in_lava = world.iter_blocks_in_box(lava_bb)
        .any(|(_, block, _)| block::from_id(block).material == Material::Lava);

    // If this entity can pickup other ones, trigger an event.
    if base.can_pickup {

        PICKED_UP_ENTITIES.with_borrow_mut(|picked_up_entities| {

            debug_assert!(picked_up_entities.is_empty());
            
            for (entity_id, entity, _) in world.iter_entities_colliding(base.bb.inflate(DVec3::new(1.0, 0.0, 1.0))) {

                match &entity.1 {
                    BaseKind::Item(item) => {
                        if item.frozen_ticks == 0 {
                            picked_up_entities.push(entity_id);
                        }
                    }
                    BaseKind::Projectile(projectile, ProjectileKind::Arrow(_)) => {
                        if projectile.block_hit.is_some() {
                            picked_up_entities.push(entity_id);
                        }
                    }
                    _ => {}
                }
            }

            for entity_id in picked_up_entities.drain(..) {
                world.push_event(Event::Entity { 
                    id, 
                    inner: EntityEvent::Pickup { 
                        target_id: entity_id,
                    },
                });
            }

        });

    }

    // If this entity is living, there is more to do.
    if let BaseKind::Living(living, _) = base_kind {
        tick_living_state(world, id, base, living);
    }

}

/// Common method for moving an entity by a given amount while checking collisions.
/// 
/// REF: Entity::moveEntity
fn tick_base_pos(world: &mut World, _id: u32, base: &mut Base, delta: DVec3, step_height: f32) {

    if base.no_clip {
        base.bb += delta;
    } else {

        // TODO: 

        // TODO: If in cobweb:
        // delta *= DVec3::new(0.25, 0.05, 0.25)
        // base.vel = DVec3::ZERO

        // TODO: Sneaking on ground

        let colliding_bb = base.bb.expand(delta);
        let colliding_bbs: Vec<_> = world.iter_blocks_boxes_colliding(colliding_bb)
            .chain(world.iter_entities_colliding(colliding_bb)
                .filter_map(|(_entity_id, entity, entity_bb)| {
                    // Only the boat entity acts like a hard bounding box.
                    if let Entity(_, BaseKind::Boat(_)) = entity {
                        Some(entity_bb)
                    } else {
                        None
                    }
                }))
            .collect();
        
        // Compute a new delta that doesn't collide with above boxes.
        let mut new_delta = delta;

        // Check collision on Y axis.
        for colliding_bb in &colliding_bbs {
            new_delta.y = colliding_bb.calc_y_delta(base.bb, new_delta.y);
        }

        base.bb += DVec3::new(0.0, new_delta.y, 0.0);

        // Check collision on X axis.
        for colliding_bb in &colliding_bbs {
            new_delta.x = colliding_bb.calc_x_delta(base.bb, new_delta.x);
        }

        base.bb += DVec3::new(new_delta.x, 0.0, 0.0);

        // Check collision on Z axis.
        for colliding_bb in &colliding_bbs {
            new_delta.z = colliding_bb.calc_z_delta(base.bb, new_delta.z);
        }
        
        base.bb += DVec3::new(0.0, 0.0, new_delta.z);

        let collided_x = delta.x != new_delta.x;
        let collided_y = delta.y != new_delta.y;
        let collided_z = delta.z != new_delta.z;
        let on_ground = collided_y && delta.y < 0.0; // || self.on_ground

        // Apply step if relevant.
        if step_height > 0.0 && on_ground && (collided_x || collided_z) {
            // TODO: todo!("handle step motion");
        }

        base.on_ground = on_ground;

        if on_ground {
            if base.fall_distance > 0.0 {
                // TODO: Damage?
            }
            base.fall_distance = 0.0;
        } else if new_delta.y < 0.0 {
            base.fall_distance -= new_delta.y as f32;
        }

        if collided_x {
            base.vel.x = 0.0;
            base.vel_dirty = true;
        }

        if collided_y {
            base.vel.y = 0.0;
            base.vel_dirty = true;
        }

        if collided_z {
            base.vel.z = 0.0;
            base.vel_dirty = true;
        }

    }

    base.update_pos_from_bounding_box();

}

/// REF: EntityItem::onUpdate
fn tick_item(world: &mut World, id: u32, base: &mut Base, item: &mut Item) {

    if item.frozen_ticks > 0 {
        item.frozen_ticks -= 1;
    }

    // Update item velocity.
    base.vel_dirty = true;
    base.vel.y -= 0.04;

    // If the item is in lava, apply random motion like it's burning.
    // NOTE: The real client don't use 'in_lava', check if problematic.
    if base.in_lava {
        base.vel.y = 0.2;
        base.vel.x = ((base.rand.next_float() - base.rand.next_float()) * 0.2) as f64;
        base.vel.z = ((base.rand.next_float() - base.rand.next_float()) * 0.2) as f64;
    }

    // If the item is in an opaque block.
    let block_pos = base.pos.floor().as_ivec3();
    if world.is_block_opaque_cube(block_pos) {

        let delta = base.pos - block_pos.as_dvec3();

        // Find a block face where we can bump the item.
        let bump_face = Face::ALL.into_iter()
            .filter(|face| !world.is_block_opaque_cube(block_pos + face.delta()))
            .map(|face| {
                let mut delta = delta[face.axis_index()];
                if face.is_pos() {
                    delta = 1.0 - delta;
                }
                (face, delta)
            })
            .min_by(|&(_, delta1), &(_, delta2)| delta1.total_cmp(&delta2))
            .map(|(face, _)| face);

        // If we found a non opaque face then we bump the item to that face.
        if let Some(bump_face) = bump_face {
            let accel = (base.rand.next_float() * 0.2 + 0.1) as f64;
            if bump_face.is_neg() {
                base.vel[bump_face.axis_index()] = -accel;
            } else {
                base.vel[bump_face.axis_index()] = accel;
            }
        }
        
    }

    // Move the item while checking collisions if needed.
    tick_base_pos(world, id, base, base.vel, 0.0);

    let mut slipperiness = 0.98;

    if base.on_ground {

        slipperiness = 0.1 * 0.1 * 58.8;

        let ground_pos = IVec3 {
            x: base.pos.x.floor() as i32,
            y: base.bb.min.y.floor() as i32 - 1,
            z: base.pos.z.floor() as i32,
        };

        if let Some((ground_id, _)) = world.get_block(ground_pos) {
            if ground_id != block::AIR {
                slipperiness = block::material::get_slipperiness(ground_id);
            }
        }

    }

    // Slow its velocity depending on ground slipperiness.
    base.vel.x *= slipperiness as f64;
    base.vel.y *= 0.98;
    base.vel.z *= slipperiness as f64;
    
    if base.on_ground {
        base.vel.y *= -0.5;
    }

    // Kill the item self after 5 minutes (5 * 60 * 20).
    if base.lifetime >= 6000 {
        world.remove_entity(id);
    }

}

/// REF: EntityPainting::onUpdate
fn tick_painting(_world: &mut World, _id: u32, _base: &mut Base, painting: &mut Painting) {
    painting.check_valid_time += 1;
    if painting.check_valid_time >= 100 {
        painting.check_valid_time = 0;
        // TODO: check painting validity and destroy it if not valid
    }
}

/// REF: EntityFallingSand::onUpdate
fn tick_falling_block(world: &mut World, id: u32, base: &mut Base, falling_block: &mut FallingBlock) {

    if falling_block.block_id == 0 {
        world.remove_entity(id);
        return;
    }

    base.vel_dirty = true;
    base.vel.y -= 0.04;

    tick_base_pos(world, id, base, base.vel, 0.0);

    if base.on_ground {

        base.vel *= DVec3::new(0.7, -0.5, 0.7);
        world.remove_entity(id);

        let block_pos = base.pos.floor().as_ivec3();
        if world.can_place_block(block_pos, Face::PosY, falling_block.block_id) {
            world.set_block_notify(block_pos, falling_block.block_id, 0);
        } else {
            world.spawn_loot(base.pos, ItemStack::new_block(falling_block.block_id, 0), 0.0);
        }

    } else if base.lifetime > 100 {
        world.remove_entity(id);
        world.spawn_loot(base.pos, ItemStack::new_block(falling_block.block_id, 0), 0.0);
    }

}

/// REF: EntityLiving::onUpdate
fn  tick_living(world: &mut World, id: u32, base: &mut Base, living: &mut Living, living_kind: &mut LivingKind) {

    fn path_weight_animal(world: &mut World, pos: IVec3) -> f32 {
        if world.is_block(pos - IVec3::Y, block::GRASS) {
            10.0
        } else {
            world.get_brightness(pos).unwrap_or(0.0) - 0.5
        }
    }

    const ANIMAL_MOVE_SPEED: f32 = 0.7;

    match living_kind {
        LivingKind::Player(_) => (),  // For now we do nothing.
        LivingKind::Ghast(_) => todo!(),
        LivingKind::Slime(_) => todo!(),
        LivingKind::Pig(_) => tick_creature_ai(world, id, base, living, ANIMAL_MOVE_SPEED, path_weight_animal),
        LivingKind::Chicken(_) => tick_creature_ai(world, id, base, living, ANIMAL_MOVE_SPEED, path_weight_animal),
        LivingKind::Cow(_) => tick_creature_ai(world, id, base, living, ANIMAL_MOVE_SPEED, path_weight_animal),
        LivingKind::Sheep(_) => tick_creature_ai(world, id, base, living, ANIMAL_MOVE_SPEED, path_weight_animal),
        LivingKind::Squid(_) => todo!(),
        LivingKind::Wolf(_) => todo!(),
        LivingKind::Creeper(_) => todo!(),
        LivingKind::Giant(_) => todo!(),
        LivingKind::PigZombie(_) => todo!(),
        LivingKind::Skeleton(_) => todo!(),
        LivingKind::Spider(_) => todo!(),
        LivingKind::Zombie(_) => todo!(),
    }

    if living.jumping {
        if base.in_water || base.in_lava {
            base.vel_dirty = true;
            base.vel.y += 0.04;
        } else if base.on_ground {
            base.vel_dirty = true;
            base.vel.y += 0.42 + 0.1; // FIXME: Added 0.1 to make it work
        }
    }

    living.accel_strafing *= 0.98;
    living.accel_forward *= 0.98;
    living.yaw_velocity *= 0.9;

    // FIXME: Exception for now to avoid player position being touched.
    if let LivingKind::Player(_) = living_kind {
        return;
    }

    tick_living_pos(world, id, base, living, living_kind);

    // TODO: Entity collision.

}

/// REF: EntityLiving::moveEntityWithHeading
fn tick_living_pos(world: &mut World, id: u32, base: &mut Base, living: &mut Living, living_kind: &mut LivingKind) {

    // Squid has no special rule for moving.
    if let LivingKind::Squid(_) = living_kind {
        tick_base_pos(world, id, base, base.vel, 0.5);
        return;
    }

    // All living entities have step height 0.5;
    let step_height = 0.5;

    // REF: EntityFlying::moveEntityWithHeading
    let flying = matches!(living_kind, LivingKind::Ghast(_));

    if base.in_water {
        tick_living_vel(world, id, base, living, 0.02);
        tick_base_pos(world, id, base, base.vel, step_height);
        base.vel *= 0.8;
        if !flying {
            base.vel.y -= 0.02;
        }
        // TODO: If collided horizontally
    } else if base.in_lava {
        tick_living_vel(world, id, base, living, 0.02);
        tick_base_pos(world, id, base, base.vel, step_height);
        base.vel *= 0.5;
        if !flying {
            base.vel.y -= 0.02;
        }
        // TODO: If collided horizontally
    } else {

        let mut slipperiness = 0.91;

        if base.on_ground {
            slipperiness = 546.0 * 0.1 * 0.1 * 0.1;
            let ground_pos = base.pos.as_ivec3();
            if let Some((ground_id, _)) = world.get_block(ground_pos) {
                if ground_id != 0 {
                    slipperiness = block::material::get_slipperiness(ground_id) * 0.91;
                }
            }
        }

        // Change entity velocity if on ground or not.
        let vel_factor = match base.on_ground {
            true => 0.16277136 / (slipperiness * slipperiness * slipperiness) * 0.1,
            false => 0.02,
        };

        tick_living_vel(world, id, base, living, vel_factor);
        
        // TODO: Is on ladder

        tick_base_pos(world, id, base, base.vel, step_height);

        // TODO: Collided horizontally and on ladder

        if flying {
            base.vel *= slipperiness as f64;
        } else {
            base.vel.y -= 0.08;
            base.vel.y *= 0.98;
            base.vel.x *= slipperiness as f64;
            base.vel.z *= slipperiness as f64;
        }

    }

    base.vel_dirty = true;
    
}

/// Update a living entity velocity according to its strafing/forward accel.
fn tick_living_vel(_world: &mut World, _id: u32, base: &mut Base, living: &mut Living, factor: f32) {

    let mut strafing = living.accel_strafing;
    let mut forward = living.accel_forward;
    let mut dist = Vec2::new(forward, strafing).length();
    if dist >= 0.01 {
        dist = dist.max(1.0);
        dist = factor / dist;
        strafing *= dist;
        forward *= dist;
        let (yaw_sin, yaw_cos) = base.look.x.sin_cos();
        base.vel_dirty = true;
        base.vel.x += (strafing * yaw_cos - forward * yaw_sin) as f64;
        base.vel.z += (forward * yaw_cos + strafing * yaw_sin) as f64;
    }
    
}

/// REF: EntityLiving::onEntityUpdate
fn tick_living_state(world: &mut World, id: u32, base: &mut Base, living: &mut Living) {

    // TODO: Damage entity if inside block

    living.attack_time = living.attack_time.saturating_sub(1);
    living.hurt_time = living.hurt_time.saturating_sub(1);

    if base.health == 0 {
        living.death_time += 1;
        if living.death_time > 20 {
            world.remove_entity(id);
        }
    }

}

/// REF: EntityLiving::updatePlayerActionState
fn tick_living_ai(world: &mut World, _id: u32, base: &mut Base, living: &mut Living) {

    // TODO: Handle kill when closest player is too far away.

    living.accel_strafing = 0.0;
    living.accel_forward = 0.0;

    // Maximum of 8 block to look at.
    let look_target_range_squared = 8.0 * 8.0;

    if base.rand.next_float() < 0.02 {
        // TODO: Look at closest player (max 8 blocks).
    }

    // If the entity should have a target, just look at it if possible, and stop if
    // the target should end or is too far away.
    if let Some(target) = &mut living.look_target {

        target.ticks_remaining -= 1;
        let mut target_release = target.ticks_remaining == 0;

        if let Some(target_entity) = world.get_entity(target.entity_id) {
            // TODO: Fix the Y value, in order to look at eye height.
            let target_pos = target_entity.0.pos;
            // TODO: Pitch step should be an argument, 40 by default, but 20 for 
            // sitting dogs.
            base.update_look_at_by_step(target_pos, Vec2::new(10f32.to_radians(), 40f32.to_radians()));
            // Indicate if the entity is still in range.
            if target_pos.distance_squared(base.pos) > look_target_range_squared {
                target_release = false;
            }
        } else {
            // Entity is dead.
            target_release = false;
        }

        if target_release {
            living.look_target = None;
        }

    } else {

        if base.rand.next_float() < 0.05 {
            living.yaw_velocity = (base.rand.next_float() - 0.5) * 20f32.to_radians();
        }

        base.look.x += living.yaw_velocity;
        base.look.y = 0.0;
        base.look_dirty = true;

    }

    if base.in_water || base.in_lava {
        living.jumping = base.rand.next_float() < 0.8;
    }

}

/// Tick an creature (animal/mob) entity AI.
/// 
/// REF: EntityCreature::updatePlayerActionState
fn tick_creature_ai(world: &mut World, id: u32, base: &mut Base, living: &mut Living, 
    move_speed: f32, 
    weight_func: fn(&mut World, IVec3) -> f32,
) {

    // TODO: Work on mob AI with attacks...

    // If the path is not none, try finding a new path every second on average.
    if living.path.is_none() || base.rand.next_int_bounded(20) != 0 {
        // Find a new path every 4 seconds on average.
        if base.rand.next_int_bounded(80) == 0 {

            if living.path.is_none() {
                trace!("entity #{id}, path finding because path none");
            } else {
                trace!("entity #{id}, path finding because 5% chance");
            }
            
            let best_pos = (0..10)
                .map(|_| {
                    IVec3 {
                        x: base.pos.x.add((base.rand.next_int_bounded(13) - 6) as f64).floor() as i32,
                        y: base.pos.y.add((base.rand.next_int_bounded(7) - 3) as f64).floor() as i32,
                        z: base.pos.z.add((base.rand.next_int_bounded(13) - 6) as f64).floor() as i32,
                    }
                })
                .map(|pos| (pos, weight_func(world, pos)))
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .unwrap().0;

            trace!("entity #{id}, path finding: {best_pos}");

            let best_pos = best_pos.as_dvec3() + 0.5;
            if let Some(points) = PathFinder::new(world).find_path_from_bounding_box(base.bb, best_pos, 18.0) {
                // println!("== update_creature_path: new path found to {best_pos}");
                trace!("entity #{id}, path found: {points:?}");
                living.path = Some(Path {
                    points,
                    index: 0,
                });
            }
                
        }
    }

    if let Some(path) = &mut living.path {

        // Debug particles, lava = remaining, water = done.
        if base.lifetime % 10 == 0 {
            for (i, pos) in path.points.iter().copied().enumerate() {
                if i < path.index {
                    world.push_event(Event::DebugParticle { pos, block: block::WATER_STILL });
                } else {
                    world.push_event(Event::DebugParticle { pos, block: block::LAVA_STILL });
                }
            }
        }

        if base.rand.next_int_bounded(100) != 0 {

            let bb_size = base.bb.size();
            let double_width = bb_size.x * 2.0;

            let mut next_pos = None;
            
            while let Some(pos) = path.point() {

                let mut pos = pos.as_dvec3();
                pos.x += (bb_size.x + 1.0) * 0.5;
                pos.z += (bb_size.z + 1.0) * 0.5;

                // Advance the path to the next point only if distance to current one is 
                // too short. We only check the horizontal distance, because Y delta is 0.
                let pos_dist_sq = pos.distance_squared(DVec3::new(base.pos.x, pos.y, base.pos.z));
                if pos_dist_sq < double_width * double_width {
                    trace!("entity #{id}, path pos to short: {pos}, dist: {} < {}", pos_dist_sq.sqrt(), double_width);
                    path.advance();
                } else {
                    next_pos = Some(pos);
                    break;
                }

            }

            living.jumping = false;

            if let Some(next_pos) = next_pos {

                trace!("entity #{id}, path next pos: {next_pos}");

                let dx = next_pos.x - base.pos.x;
                let dy = next_pos.y - base.bb.min.y.add(0.5).floor();
                let dz = next_pos.z - base.pos.z;

                let target_yaw = f64::atan2(dz, dx) as f32 - std::f32::consts::FRAC_PI_2;
                // let delta_yaw = target_yaw - base.look.x;

                living.accel_forward = move_speed;
                base.look.x = target_yaw;
                base.look_dirty = true;

                if dy > 0.0 {
                    living.jumping = true;
                }

            } else {
                trace!("entity #{id}, path finished");
                living.path = None;
            }

            // TODO: If player to attack

            // TODO: If collided horizontal and no path, then jump

            if base.rand.next_float() < 0.8 && (base.in_water || base.in_lava) {
                trace!("entity #{id}, jumping because of 80% chance or water/lava");
                living.jumping = true;
            }

            return;

        } else {
            trace!("entity #{id}, forget path because 1% chance")
        }

    }

    // If we can't run a path finding AI, fallback to the default immobile AI.
    living.path = None;
    tick_living_ai(world, id, base, living);

}


/// Calculate the initial size of an entity, this is only called when not coherent.
fn calc_size(base_kind: &mut BaseKind) -> Size {
    match base_kind {
        BaseKind::Item(_) => Size::new_centered(0.25, 0.25),
        BaseKind::Painting(_) => Size::new(0.5, 0.5),
        BaseKind::Boat(_) => Size::new_centered(1.5, 0.6),
        BaseKind::Minecart(_) => Size::new_centered(0.98, 0.7),
        BaseKind::Fish(_) => Size::new(0.25, 0.25),
        BaseKind::LightningBolt(_) => Size::new(0.0, 0.0),
        BaseKind::FallingBlock(_) => Size::new_centered(0.98, 0.98),
        BaseKind::Tnt(_) => Size::new_centered(0.98, 0.98),
        BaseKind::Projectile(_, ProjectileKind::Arrow(_)) => Size::new(0.5, 0.5),
        BaseKind::Projectile(_, ProjectileKind::Egg(_)) =>Size::new(0.5, 0.5),
        BaseKind::Projectile(_, ProjectileKind::Fireball(_)) => Size::new(1.0, 1.0),
        BaseKind::Projectile(_, ProjectileKind::Snowball(_)) => Size::new(0.5, 0.5),
        BaseKind::Living(_, LivingKind::Player(player)) => {
            if player.sleeping {
                Size::new(0.2, 0.2)
            } else {
                Size::new(0.6, 1.8)
            }
        }
        BaseKind::Living(_, LivingKind::Ghast(_)) => Size::new(4.0, 4.0),
        BaseKind::Living(_, LivingKind::Slime(slime)) => {
            let factor = slime.size as f32;
            Size::new(0.6 * factor, 0.6 * factor)
        }
        BaseKind::Living(_, LivingKind::Pig(_)) => Size::new(0.9, 0.9),
        BaseKind::Living(_, LivingKind::Chicken(_)) => Size::new(0.3, 0.4),
        BaseKind::Living(_, LivingKind::Cow(_)) => Size::new(0.9, 1.3),
        BaseKind::Living(_, LivingKind::Sheep(_)) =>Size::new(0.9, 1.3),
        BaseKind::Living(_, LivingKind::Squid(_)) => Size::new(0.95, 0.95),
        BaseKind::Living(_, LivingKind::Wolf(_)) => Size::new(0.8, 0.8),
        BaseKind::Living(_, LivingKind::Creeper(_)) => Size::new(0.6, 1.8),
        BaseKind::Living(_, LivingKind::Giant(_)) => Size::new(3.6, 10.8),
        BaseKind::Living(_, LivingKind::PigZombie(_)) => Size::new(0.6, 1.8),
        BaseKind::Living(_, LivingKind::Skeleton(_)) => Size::new(0.6, 1.8),
        BaseKind::Living(_, LivingKind::Spider(_)) => Size::new(1.4, 0.9),
        BaseKind::Living(_, LivingKind::Zombie(_)) => Size::new(0.6, 1.8),
    }
}
