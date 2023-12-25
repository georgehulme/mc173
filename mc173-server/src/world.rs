//! Server world structure.

use std::collections::HashMap;
use std::time::Instant;

use glam::{DVec3, IVec3, Vec2};

use tracing::{debug, info, instrument};

use mc173::chunk::calc_entity_chunk_pos;
use mc173::entity::{Entity, BaseKind, ProjectileKind};
use mc173::storage::{ChunkStorage, ChunkStorageReply};
use mc173::gen::OverworldGenerator;
use mc173::item::{ItemStack, self};
use mc173::{chunk, block};

use mc173::world::{World, Dimension, 
    Event, EntityEvent, BlockEntityEvent, BlockEvent, 
    BlockEntityStorage, BlockEntityProgress, 
    Weather};

use crate::proto::{self, OutPacket};
use crate::entity::EntityTracker;
use crate::player::ServerPlayer;
use crate::chunk::ChunkTrackers;


/// Server world seed is currently hardcoded.
const SEED: i64 = 9999;


/// A single world in the server, this structure keep tracks of players and entities
/// tracked by players.
pub struct ServerWorld {
    /// The inner world data structure.
    pub world: World,
    /// Players currently in the world.
    pub players: Vec<ServerPlayer>,
    /// The remaining world state, this is put is a separate struct in order to facilitate
    /// borrowing when handling player packets.
    pub state: ServerWorldState,
}

/// Represent the whole state of a world.
pub struct ServerWorldState {
    /// World name.
    pub name: String,
    /// The seed of this world, this is sent to the client in order to 
    pub seed: i64,
    /// The server-side time, that is not necessarily in-sync with the world time in case
    /// of tick freeze or stepping. This avoids running in socket timeout issues.
    pub time: u64,
    /// True when world ticking is frozen, events are still processed by the world no 
    /// longer runs.
    pub tick_mode: TickMode,
    /// The chunk source used to load and save the world's chunk.
    storage: ChunkStorage,
    /// Chunks trackers used to send proper block changes packets.
    chunk_trackers: ChunkTrackers,
    /// Entity tracker, each is associated to the entity id.
    entity_trackers: HashMap<u32, EntityTracker>,
    /// Sliding average tick duration, in seconds.
    tick_duration: f32,
    /// Sliding average interval between two ticks.
    tick_interval: f32,
    /// Instant of the last tick.
    tick_last: Instant,
}

/// Indicate the current mode for ticking the world.
pub enum TickMode {
    /// The world is ticked on each server tick (20 TPS).
    Auto,
    /// The world if ticked on each server tick (20 TPS), but the counter decrease and
    /// it is no longer ticked when reaching 0.
    Manual(u32),
}

impl ServerWorld {

    /// Internal function to create a server world.
    pub fn new(name: impl Into<String>) -> Self {

        let mut inner = World::new(Dimension::Overworld);

        // Make sure that the world initially have an empty events queue.
        inner.swap_events(Some(Vec::new()));

        let seed = SEED;
        
        Self {
            world: inner,
            players: Vec::new(),
            state: ServerWorldState {
                name: name.into(),
                seed,
                time: 0,
                tick_mode: TickMode::Auto,
                storage: ChunkStorage::new("test_world/region/", OverworldGenerator::new(seed), 4),
                chunk_trackers: ChunkTrackers::new(),
                entity_trackers: HashMap::new(),
                tick_duration: 0.0,
                tick_interval: 0.0,
                tick_last: Instant::now(),
            },
        }

    }

    /// Save this world's resources and block until all resources has been saved.
    pub fn save(&mut self) {

        info!("saving {}...", self.state.name);

        for (cx, cz) in self.state.chunk_trackers.drain_save() {
            if let Some(snapshot) = self.world.take_chunk_snapshot(cx, cz) {
                debug!("saving {} chunk: {cx}/{cz}", self.state.name);
                self.state.storage.request_save(snapshot);
            }
        }

        while self.state.storage.request_save_count() != 0 {
            let _ = self.state.storage.poll();
        }

    }

    /// Tick this world.
    #[instrument(skip(self))]
    pub fn tick(&mut self) {

        let start = Instant::now();
        self.state.tick_interval = (self.state.tick_interval * 0.98) + (start - self.state.tick_last).as_secs_f32() * 0.02;
        self.state.tick_last = start;

        // Get server-side time.
        let time = self.state.time;
        if time == 0 {
            self.init();
        }

        // Poll all chunks to load in the world.
        while let Some(reply) = self.state.storage.poll() {
            match reply {
                ChunkStorageReply::Load { cx, cz, res: Ok(snapshot) } => {
                    debug!("loaded chunk from storage: {cx}/{cz}");
                    self.world.insert_chunk_snapshot(snapshot);
                }
                ChunkStorageReply::Load { cx, cz, res: Err(err) } => {
                    debug!("failed to load chunk from storage: {cx}/{cz}: {err}");
                }
                ChunkStorageReply::Save { cx, cz, res: Ok(()) } => {
                    debug!("saved chunk in storage: {cx}/{cz}");
                }
                ChunkStorageReply::Save { cx, cz, res: Err(err) } => {
                    debug!("failed to save chunk in storage: {cx}/{cz}: {err}");
                }
            }
        }

        // Only run if no tick freeze.
        match self.state.tick_mode {
            TickMode::Auto => {
                self.world.tick()
            }
            TickMode::Manual(0) => {}
            TickMode::Manual(ref mut n) => {
                self.world.tick();
                *n -= 1;
            }
        }

        // Swap events out in order to proceed them.
        let mut events = self.world.swap_events(None).expect("events should be enabled");
        for event in events.drain(..) {
            match event {
                Event::Block { pos, inner } => match inner {
                    BlockEvent::Set { id, metadata, prev_id, prev_metadata } =>
                        self.handle_block_set(pos, id, metadata, prev_id, prev_metadata),
                    BlockEvent::Sound { id, metadata } =>
                        self.handle_block_sound(pos, id, metadata),
                }
                Event::Entity { id, inner } => match inner {
                    EntityEvent::Spawn => 
                        self.handle_entity_spawn(id),
                    EntityEvent::Remove => 
                        self.handle_entity_remove(id),
                    EntityEvent::Position { pos } => 
                        self.handle_entity_position(id, pos),
                    EntityEvent::Look { look } => 
                        self.handle_entity_look(id, look),
                    EntityEvent::Velocity { vel } => 
                        self.handle_entity_velocity(id, vel),
                    EntityEvent::Pickup { target_id } => 
                        self.handle_entity_pickup(id, target_id),
                    EntityEvent::Damage => 
                        self.handle_entity_damage(id),
                    EntityEvent::Dead => 
                        self.handle_entity_dead(id),
                    EntityEvent::Creeper { ignited, powered } =>
                        todo!(),
                    EntityEvent::Ghast { charged } =>
                        self.handle_entity_ghast(id, charged),
                }
                Event::BlockEntity { pos, inner } => match inner {
                    BlockEntityEvent::Set =>
                        self.handle_block_entity_set(pos),
                    BlockEntityEvent::Remove =>
                        self.handle_block_entity_remove(pos),
                    BlockEntityEvent::Storage { storage, stack } =>
                        self.handle_block_entity_storage(pos, storage, stack),
                    BlockEntityEvent::Progress { progress, value } =>
                        self.handle_block_entity_progress(pos, progress, value),
                }
                Event::Weather { new, .. } =>
                    self.handle_weather_change(new),
                Event::DebugParticle { pos, block } =>
                    self.handle_debug_particle(pos, block),
            }
        }

        // Reinsert events after processing.
        self.world.swap_events(Some(events));

        // Send time to every playing clients every second.
        if time % 20 == 0 {
            let world_time = self.world.get_time();
            for player in &self.players {
                player.send(OutPacket::UpdateTime(proto::UpdateTimePacket {
                    time: world_time,
                }));
            }
        }

        // After we collected every block change, update all players accordingly.
        self.state.chunk_trackers.update_players(&self.players, &self.world);

        // After world events are processed, tick entity trackers.
        for tracker in self.state.entity_trackers.values_mut() {
            if time % 60 == 0 {
                tracker.update_tracking_players(&mut self.players, &self.world);
            }
            tracker.tick_and_update_players(&self.players);
        }

        // Drain dirty chunks coordinates and save them.
        while let Some((cx, cz)) = self.state.chunk_trackers.next_save() {
            if let Some(snapshot) = self.world.take_chunk_snapshot(cx, cz) {
                self.state.storage.request_save(snapshot);
            }
        }

        // Update tick duration metric.
        let tick_duration = start.elapsed();
        self.state.tick_duration = (self.state.tick_duration * 0.98) + tick_duration.as_secs_f32() * 0.02;

        // Finally increase server-side tick time.
        self.state.time += 1;

    }
    
    /// Initialize the world by ensuring that every entity is currently tracked. This
    /// method can be called multiple time and should be idempotent.
    fn init(&mut self) {

        // Ensure that every entity has a tracker.
        for (id, entity) in self.world.iter_entities() {
            self.state.entity_trackers.entry(id).or_insert_with(|| {
                let tracker = EntityTracker::new(id, entity);
                tracker.update_tracking_players(&mut self.players, &self.world);
                tracker
            });
        }

        // FIXME: Temporary code.
        let center_cx = 0;
        let center_cz = 0;
        for cx in center_cx - 10..=center_cx + 10 {
            for cz in center_cz - 10..=center_cz + 10 {
                self.state.storage.request_load(cx, cz);
            }
        }

    }

    /// Handle a player joining this world.
    #[instrument(skip_all)]
    pub fn handle_player_join(&mut self, mut player: ServerPlayer) -> usize {

        // Initial tracked entities.
        for tracker in self.state.entity_trackers.values() {
            tracker.update_tracking_player(&mut player, &self.world);
        }

        player.update_chunks(&self.world);
        
        let player_index = self.players.len();
        self.players.push(player);
        player_index

    }

    /// Handle a player leaving this world, this should remove its entity. The `lost`
    /// argument indicates if the player is leaving because of a lost connection or not.
    /// If the connection was not lost, chunks and entities previously tracked by the
    /// player are send to be untracked. 
    /// 
    /// **Note that** this function swap remove the player, so the last player in this
    /// world's list is moved to the given player index. So if it exists, you should 
    /// update all indices pointing to the swapped player. This method returns, if 
    /// existing, the player that was swapped.
    #[instrument(skip_all)]
    pub fn handle_player_leave(&mut self, player_index: usize, lost: bool) -> Option<&ServerPlayer> {

        // Remove the player tracker.
        let mut player = self.players.swap_remove(player_index);
        
        // Kill the entity associated to the player.
        self.world.remove_entity(player.entity_id);

        // If player has not lost connection but it's just leaving the world, we just
        // send it untrack packets.
        if !lost {
            
            // Take and replace it with an empty set (no overhead).
            let tracked_entities = std::mem::take(&mut player.tracked_entities);

            // Untrack all its entities.
            for entity_id in tracked_entities {
                let tracker = self.state.entity_trackers.get(&entity_id).expect("incoherent tracked entity");
                tracker.kill_player_entity(&mut player);
            }

        }

        self.players.get(player_index)

    }

    /// Handle a block change world event.
    fn handle_block_set(&mut self, pos: IVec3, id: u8, metadata: u8, prev_id: u8, _prev_metadata: u8) {

        // Notify the tracker of the block change, this is used to notify the player 
        self.state.chunk_trackers.set_block(pos, id, metadata);

        // If the block was a crafting table, if any player has a crafting table
        // window referencing this block then we force close it.
        let break_crafting_table = id != prev_id && prev_id == block::CRAFTING_TABLE;
        if break_crafting_table {
            for player in &mut self.players {
                player.close_block_window(&mut self.world, pos);
            }
        }

    }

    fn handle_block_sound(&mut self, pos: IVec3, _block: u8, _metadata: u8) {
        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        for player in &self.players {
            if player.tracked_chunks.contains(&(cx, cz)) {
                player.send(OutPacket::EffectPlay(proto::EffectPlayPacket {
                    effect_id: 1003,
                    x: pos.x,
                    y: pos.y as i8,
                    z: pos.z,
                    effect_data: 0,
                }));
            }
        }
    }

    /// Handle an entity spawn world event.
    fn handle_entity_spawn(&mut self, id: u32) {
        
        let Some(entity) = self.world.get_entity(id) else {
            return // Entity is already removed from the world.
        };

        self.state.entity_trackers.entry(id).or_insert_with(|| {
            let tracker = EntityTracker::new(id, entity);
            tracker.update_tracking_players(&mut self.players, &self.world);
            tracker
        });

    }

    /// Handle an entity kill world event.
    fn handle_entity_remove(&mut self, id: u32) {
        
        let Some(tracker) = self.state.entity_trackers.remove(&id) else {
            return // Entity has not been added because: see above.
        };

        tracker.untrack_players(&mut self.players);

    }

    /// Handle an entity position world event.
    fn handle_entity_position(&mut self, id: u32, pos: DVec3) {
        let (cx, cz) = calc_entity_chunk_pos(pos);
        self.state.chunk_trackers.set_dirty(cx, cz);
        self.state.entity_trackers.get_mut(&id).unwrap().set_pos(pos);
    }

    /// Handle an entity look world event.
    fn handle_entity_look(&mut self, id: u32, look: Vec2) {
        self.state.entity_trackers.get_mut(&id).unwrap().set_look(look);
    }

    /// Handle an entity look world event.
    fn handle_entity_velocity(&mut self, id: u32, vel: DVec3) {
        self.state.entity_trackers.get_mut(&id).unwrap().set_vel(vel);
    }

    /// Handle an entity pickup world event.
    fn handle_entity_pickup(&mut self, id: u32, target_id: u32) {

        let Some(Entity(_, target_kind)) = self.world.get_entity_mut(target_id) else { return };
        let Some(player) = self.players.iter_mut().find(|p| p.entity_id == id) else {
            // This works only on entities handled by players.
            return
        };

        // Used only for picking arrow.
        let mut arrow_stack = ItemStack::new_single(item::ARROW, 0);
        
        let stack = match target_kind {
            BaseKind::Item(item) 
                => &mut item.stack,
            BaseKind::Projectile(projectile, ProjectileKind::Arrow(_)) 
                if projectile.shake == 0 
                => &mut arrow_stack,
            // Other entities cannot be picked up.
            _ => return,
        };

        player.pickup_stack(stack);

        // If the item stack has been emptied, kill the entity.
        if stack.size == 0 {
            self.world.remove_entity(target_id);
        }

        for player in &self.players {
            if player.tracked_entities.contains(&target_id) {
                player.send(OutPacket::EntityPickup(proto::EntityPickupPacket {
                    entity_id: id,
                    picked_entity_id: target_id,
                }));
            }
        }

    }

    /// Handle an entity damage event.
    fn handle_entity_damage(&mut self, id: u32) {

        self.handle_entity_status(id, 2);

        // TODO: This is temporary code, we need to make a common method to update health.
        for player in &self.players {
            if player.entity_id == id {
                if let Entity(_, BaseKind::Living(living, _)) = self.world.get_entity(id).unwrap() {
                    player.send(OutPacket::UpdateHealth(proto::UpdateHealthPacket {
                        health: living.health.min(i16::MAX as _) as i16,
                    }));
                }
            }
        }

    }

    /// Handle an entity dead event (the entity is not yet removed).
    fn handle_entity_dead(&mut self, id: u32) {
        self.handle_entity_status(id, 3);
    }

    /// Handle an entity damage/dead or other status for an entity.
    fn handle_entity_status(&mut self, id: u32, status: u8) {
        for player in &self.players {
            if player.tracked_entities.contains(&id) || player.entity_id == id {
                player.send(OutPacket::EntityStatus(proto::EntityStatusPacket {
                    entity_id: id,
                    status,
                }));
            }
        }
    }

    /// Handle an entity ghast state event.
    fn handle_entity_ghast(&mut self, id: u32, charged: bool) {
        for player in &self.players {
            if player.tracked_entities.contains(&id) {
                player.send(OutPacket::EntityMetadata(proto::EntityMetadataPacket {
                    entity_id: id,
                    metadata: vec![
                        proto::Metadata::new_byte(16, charged as _)
                    ],
                }));
            }
        }
    }

    /// HAndle a block entity set event.
    fn handle_block_entity_set(&mut self, pos: IVec3) {
        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        self.state.chunk_trackers.set_dirty(cx, cz);
    }

    /// Handle a block entity remove event.
    fn handle_block_entity_remove(&mut self, pos: IVec3) {

        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        self.state.chunk_trackers.set_dirty(cx, cz);
        
        // Close the inventory of all entities that had a window opened for this block.
        for player in &mut self.players {
            player.close_block_window(&mut self.world, pos);
        }

    }

    /// Handle a storage event for a block entity.
    fn handle_block_entity_storage(&mut self, pos: IVec3, storage: BlockEntityStorage, stack: ItemStack) {

        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        self.state.chunk_trackers.set_dirty(cx, cz);

        for player in &mut self.players {
            player.update_block_window_storage(pos, storage, stack);
        }

    }

    fn handle_block_entity_progress(&mut self, pos: IVec3, progress: BlockEntityProgress, value: u16) {

        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        self.state.chunk_trackers.set_dirty(cx, cz);
        
        for player in &mut self.players {
            player.update_block_window_progress(pos, progress, value);
        }

    }

    /// Handle weather change in the world.
    fn handle_weather_change(&mut self, weather: Weather) {
        for player in &self.players {
            player.send(OutPacket::Notification(proto::NotificationPacket {
                reason: if weather == Weather::Clear { 2 } else { 1 },
            }));
        }
    }

    fn handle_debug_particle(&mut self, pos: IVec3, block: u8) {
        let (cx, cz) = chunk::calc_chunk_pos_unchecked(pos);
        for player in &self.players {
            if player.tracked_chunks.contains(&(cx, cz)) {
                player.send(OutPacket::EffectPlay(proto::EffectPlayPacket {
                    effect_id: 2001,
                    x: pos.x,
                    y: pos.y as i8,
                    z: pos.z,
                    effect_data: block as u32,
                }));
            }
        }
    }

}
