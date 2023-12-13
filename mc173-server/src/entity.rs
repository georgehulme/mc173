//! Entity tracking.

use std::ops::{Mul, Div};

use glam::{DVec3, Vec2, IVec3};

use mc173::entity::{self as e, Entity, EntityKind, BaseKind, ProjectileKind, LivingKind};
use mc173::world::World;
use mc173::block;

use crate::proto::{OutPacket, self};
use crate::player::ServerPlayer;


/// This structure tracks every entity spawned in the world and save their previous 
/// position/look (and motion for some entities). It handle allows sending the right
/// packets to the right players when these properties are changed.
#[derive(Debug)]
pub struct EntityTracker {
    /// The entity id.
    id: u32,
    /// Maximum tracking distance for this type of entity.
    distance: u16,
    /// Update interval for this type of entity.
    interval: u16,
    /// Internal time of this entity tracker, this is reset to 0 when reaching interval.
    time: u16,
    /// This countdown is reset when the absolute position is sent, if the absolute 
    /// position has not been sent for 400 ticks (20 seconds), it's resent.
    absolute_countdown_time: u16,
    /// Last known position of the entity.
    pos: (i32, i32, i32),
    /// Last known look of the entity.
    look: (i8, i8),
    /// Last encoded position sent to clients.
    sent_pos: (i32, i32, i32),
    /// Last encoded look sent to clients.
    sent_look: (i8, i8),
    /// If this tracker should track entity velocity, this contains the tracker.
    vel: Option<EntityVelocityTracker>,
}

/// Some entity velocity tracking if enabled for that entity.
#[derive(Debug)]
struct EntityVelocityTracker {
    /// Last known velocity of the entity.
    vel: (i16, i16, i16),
    /// Last encoded velocity sent to clients.
    sent_vel: (i16, i16, i16),
}

impl EntityTracker {

    /// Construct an entity tracker for the given entity with its id. The tracker 
    /// configuration will be different depending on the entity kind, and the initial
    /// position and look are encoded.
    pub fn new(id: u32, entity: &Entity) -> Self {

        let (distance, interval, velocity) = match entity.kind() {
            EntityKind::Player => (512, 2, false),
            EntityKind::Fish => (64, 5, true),
            EntityKind::Arrow => (64, 20, false),
            EntityKind::Fireball => (64, 10, false),
            EntityKind::Snowball => (64, 10, true),
            EntityKind::Egg => (64, 10, true),
            EntityKind::Item => (64, 5, true), // Notchian use 20 ticks
            EntityKind::Minecart => (160, 5, true),
            EntityKind::Boat => (160, 5, true),
            EntityKind::Squid => (160, 3, true),
            EntityKind::Tnt => (160, 10, true),
            EntityKind::FallingBlock => (160, 20, true),
            EntityKind::Painting => (160, 0, false),
            // All remaining animals and mobs.
            _ => (160, 3, true)
        };

        let mut tracker = Self {
            id,
            distance,
            interval,
            time: 0,
            absolute_countdown_time: 0,
            pos: (0, 0, 0),
            look: (0, 0),
            sent_pos: (0, 0, 0),
            sent_look: (0, 0),
            vel: velocity.then_some(EntityVelocityTracker { 
                vel: (0, 0, 0),
                sent_vel: (0, 0, 0),
            }),
        };
        
        tracker.set_pos(entity.0.pos);
        tracker.set_look(entity.0.look);
        tracker.sent_pos = tracker.pos;
        tracker.sent_look = tracker.look;
        tracker

    } 

    /// Update the last known position of this tracked entity.
    pub fn set_pos(&mut self, pos: DVec3) {
        let scaled = pos.mul(32.0).floor().as_ivec3();
        self.pos = (scaled.x, scaled.y, scaled.z);
    }

    /// Update the last known look of this tracked entity.
    pub fn set_look(&mut self, look: Vec2) {
        // Rebase 0..2PI to 0..256. 
        let scaled = look.mul(256.0).div(std::f32::consts::TAU);
        // We can cast to i8, this will take the low 8 bits and wrap around.
        self.look = (scaled.x as i8, scaled.y as i8);
    }

    /// Update the last known velocity of this entity.
    pub fn set_vel(&mut self, vel: DVec3) {
        if let Some(tracker) = &mut self.vel {
            // The Notchian client clamps the input velocity, this ensure that the scaled 
            // vector is in i16 range or integers.
            let scaled = vel.clamp(DVec3::splat(-3.9), DVec3::splat(3.9)).mul(8000.0).as_ivec3();
            tracker.vel = (scaled.x as i16, scaled.y as i16, scaled.z as i16);
        }
    }

    /// Tick this entity tracker and update players if needed. Only the players that
    /// already track this entity will be updated if relevant.
    pub fn tick_and_update_players(&mut self, players: &[ServerPlayer]) {

        // If the interval is 0, then entity should not be updated after being created.
        if self.interval == 0 {
            return;
        }

        self.absolute_countdown_time += 1;
        self.time += 1;

        if self.time >= self.interval {
            self.time = 0;
            self.update_players(players);
        }

    }

    /// Update this tracker to determine which move packet to send and to which players.
    pub fn update_players(&mut self, players: &[ServerPlayer]) {

        let mut send_pos = true;
        let send_look = self.look.0.abs_diff(self.sent_look.0) >= 8 || self.look.1.abs_diff(self.sent_look.1) >= 8;

        // Check if the delta can be sent with a move packet.
        let dx = i8::try_from(self.pos.0 - self.sent_pos.0).ok();
        let dy = i8::try_from(self.pos.1 - self.sent_pos.1).ok();
        let dz = i8::try_from(self.pos.2 - self.sent_pos.2).ok();

        let mut move_packet = None;
        let forced_position = self.absolute_countdown_time > 400;

        if let (false, Some(dx), Some(dy), Some(dz)) = (forced_position, dx, dy, dz) {

            // We don't send position if delta is too small.
            send_pos = dx.abs() >= 8 || dy.abs() >= 8 || dz.abs() >= 8;

            if send_pos && send_look {
                move_packet = Some(OutPacket::EntityMoveAndLook(proto::EntityMoveAndLookPacket {
                    entity_id: self.id,
                    dx,
                    dy,
                    dz,
                    yaw: self.look.0,
                    pitch: self.look.1,
                }))
            } else if send_pos {
                move_packet = Some(OutPacket::EntityMove(proto::EntityMovePacket {
                    entity_id: self.id,
                    dx,
                    dy,
                    dz,
                }))
            } else if send_look {
                move_packet = Some(OutPacket::EntityLook(proto::EntityLookPacket {
                    entity_id: self.id,
                    yaw: self.look.0,
                    pitch: self.look.1,
                }))
            }

        } else {
            self.absolute_countdown_time = 0;
            move_packet = Some(OutPacket::EntityPositionAndLook(proto::EntityPositionAndLookPacket {
                entity_id: self.id,
                x: self.pos.0,
                y: self.pos.1,
                z: self.pos.2,
                yaw: self.look.0,
                pitch: self.look.1,
            }));
        }

        if send_pos {
            self.sent_pos = self.pos;
        }

        if send_look {
            self.sent_look = self.look;
        }

        // If velocity tracking is enabled...
        if let Some(tracker) = &mut self.vel {
            // We differ from the Notchian server because we don't check for the distance.
            let dvx = tracker.vel.0 as i32 - tracker.sent_vel.0 as i32;
            let dvy = tracker.vel.1 as i32 - tracker.sent_vel.1 as i32;
            let dvz = tracker.vel.2 as i32 - tracker.sent_vel.2 as i32;
            // If any axis velocity change by 0.0125 (100 when encoded *8000).
            if dvx.abs() > 100 || dvy.abs() > 100 || dvz.abs() > 100 {
                for player in players {
                    if player.tracked_entities.contains(&self.id) {
                        player.send(OutPacket::EntityVelocity(proto::EntityVelocityPacket {
                            entity_id: self.id,
                            vx: tracker.vel.0,
                            vy: tracker.vel.1,
                            vz: tracker.vel.2,
                        }));
                    }
                }
                tracker.sent_vel = tracker.vel;
            }
        }

        if let Some(packet) = move_packet {
            for player in players {
                if player.tracked_entities.contains(&self.id) {
                    player.send(packet.clone());
                }
            }
        }

    }

    /// Update players to track or untrack this entity. See [`update_tracking_player`].
    pub fn update_tracking_players(&self, players: &mut [ServerPlayer], world: &World) {
        for player in players {
            self.update_tracking_player(player, world);
        }
    }

    /// Update a player to track or untrack this entity. The correct packet is sent if
    /// the entity needs to appear or disappear on the client side.
    pub fn update_tracking_player(&self, player: &mut ServerPlayer, world: &World) {
        
        // A player cannot track its own entity.
        if player.entity_id == self.id {
            return;
        }

        let delta = player.pos - IVec3::new(self.pos.0, self.pos.1, self.pos.2).as_dvec3() / 32.0;
        if delta.x.abs() <= self.distance as f64 && delta.z.abs() <= self.distance as f64 {
            if player.tracked_entities.insert(self.id) {
                self.spawn_player_entity(player, world);
            }
        } else if player.tracked_entities.remove(&self.id) {
            self.kill_player_entity(player);
        }

    }

    /// Force untrack this entity to this player if the player is already tracking it.
    pub fn untrack_player(&self, player: &mut ServerPlayer) {
        if player.tracked_entities.remove(&self.id) {
            self.kill_player_entity(player);
        }
    }

    /// Force untrack this entity to all given players, it applies only to players that
    /// were already tracking the entity.
    pub fn untrack_players(&self, players: &mut [ServerPlayer]) {
        for player in players {
            self.untrack_player(player);
        }
    }

    /// Spawn the entity on the player side.
    pub fn spawn_player_entity(&self, player: &ServerPlayer, world: &World) {

        // NOTE: Silently ignore dead if the entity is dead, it will be killed later.
        let Some(Entity(base, base_kind)) = world.get_entity(self.id) else { return };
        
        match base_kind {
            BaseKind::Item(item) => self.spawn_player_entity_item(player, base, item),
            BaseKind::Painting(_) => todo!(),  // TODO:
            BaseKind::Boat(_) => self.spawn_player_entity_object(player, 1, false),
            BaseKind::Minecart(e::Minecart::Normal) => self.spawn_player_entity_object(player, 10, false),
            BaseKind::Minecart(e::Minecart::Chest { .. }) => self.spawn_player_entity_object(player, 11, false),
            BaseKind::Minecart(e::Minecart::Furnace { .. }) => self.spawn_player_entity_object(player, 12, false),
            BaseKind::Fish(_) => self.spawn_player_entity_object(player, 90, false),
            BaseKind::LightningBolt(_) => (),
            BaseKind::FallingBlock(falling_block) => {
                // NOTE: We use sand for any block id that is unsupported.
                match falling_block.block_id {
                    block::GRAVEL => self.spawn_player_entity_object(player, 71, false),
                    _ => self.spawn_player_entity_object(player, 70, false),
                }
            }
            BaseKind::Tnt(_) => self.spawn_player_entity_object(player, 50, false),
            BaseKind::Projectile(_, projectile_kind) => {
                match projectile_kind {
                    ProjectileKind::Arrow(_) => self.spawn_player_entity_object(player, 60, true),
                    ProjectileKind::Egg(_) => self.spawn_player_entity_object(player, 62, false),
                    ProjectileKind::Fireball(_) => self.spawn_player_entity_object(player, 63, true),
                    ProjectileKind::Snowball(_) => self.spawn_player_entity_object(player, 61, false),
                }
            }
            BaseKind::Living(_, living_kind) => {
                match living_kind {
                    LivingKind::Player(pl) => self.spawn_player_entity_player(player, pl),
                    LivingKind::Ghast(_) => self.spawn_player_entity_mob(player, 56),
                    LivingKind::Slime(_) => self.spawn_player_entity_mob(player, 55),
                    LivingKind::Pig(_) => self.spawn_player_entity_mob(player, 90),
                    LivingKind::Chicken(_) => self.spawn_player_entity_mob(player, 93),
                    LivingKind::Cow(_) => self.spawn_player_entity_mob(player, 92),
                    LivingKind::Sheep(_) => self.spawn_player_entity_mob(player, 91),
                    LivingKind::Squid(_) => self.spawn_player_entity_mob(player, 94),
                    LivingKind::Wolf(_) => self.spawn_player_entity_mob(player, 95),
                    LivingKind::Creeper(_) => self.spawn_player_entity_mob(player, 50),
                    LivingKind::Giant(_) => self.spawn_player_entity_mob(player, 53),
                    LivingKind::PigZombie(_) => self.spawn_player_entity_mob(player, 57),
                    LivingKind::Skeleton(_) => self.spawn_player_entity_mob(player, 51),
                    LivingKind::Spider(_) => self.spawn_player_entity_mob(player, 52),
                    LivingKind::Zombie(_) => self.spawn_player_entity_mob(player, 54),
                }
            }
        }

    }

    fn spawn_player_entity_player(&self, player: &ServerPlayer, pl: &e::Player) {
        player.send(OutPacket::PlayerSpawn(proto::PlayerSpawnPacket {
            entity_id: self.id,
            username: pl.username.clone(),
            x: self.sent_pos.0, 
            y: self.sent_pos.1, 
            z: self.sent_pos.2, 
            yaw: self.sent_look.0,
            pitch: self.sent_look.1,
            current_item: 0, // TODO:
        }));
    }

    fn spawn_player_entity_item(&self, player: &ServerPlayer, base: &e::Base, item: &e::Item) {
        let vel = base.vel.mul(128.0).as_ivec3();
        player.send(OutPacket::ItemSpawn(proto::ItemSpawnPacket { 
            entity_id: self.id, 
            stack: item.stack, 
            x: self.sent_pos.0, 
            y: self.sent_pos.1, 
            z: self.sent_pos.2, 
            vx: vel.x as i8,
            vy: vel.y as i8,
            vz: vel.z as i8,
        }));
    }

    fn spawn_player_entity_object(&self, player: &ServerPlayer, kind: u8, vel: bool) {
        player.send(OutPacket::ObjectSpawn(proto::ObjectSpawnPacket {
            entity_id: self.id,
            kind,
            x: self.sent_pos.0, 
            y: self.sent_pos.1, 
            z: self.sent_pos.2, 
            velocity: vel.then(|| {
                self.vel.as_ref().expect("expected velocity to be tracked").sent_vel
            })
        }));
    }

    fn spawn_player_entity_mob(&self, player: &ServerPlayer, kind: u8) {
        player.send(OutPacket::MobSpawn(proto::MobSpawnPacket {
            entity_id: self.id,
            kind,
            x: self.sent_pos.0, 
            y: self.sent_pos.1, 
            z: self.sent_pos.2, 
            yaw: self.sent_look.0,
            pitch: self.sent_look.1,
            metadata: vec![], // TODO:
        }));
    }

    /// Kill the entity on the player side.
    pub fn kill_player_entity(&self, player: &ServerPlayer) {
        player.send(OutPacket::EntityKill(proto::EntityKillPacket { 
            entity_id: self.id
        }));
    }

}