//! The network server managing connected players and dispatching incoming packets.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use mc173::entity::{self as e};
use mc173::world::{Dimension, Weather};

use crate::config;
use crate::offline::OfflinePlayer;
use crate::player::ServerPlayer;
use crate::proto::{self, InPacket, Network, NetworkClient, NetworkEvent, OutPacket};
use crate::world::ServerWorld;

/// Target tick duration. Currently 20 TPS, so 50 ms/tick.
const TICK_DURATION: Duration = Duration::from_millis(50);

/// This structure manages a whole server and its clients, dispatching incoming packets
/// to correct handlers. The server is responsible of associating clients
pub struct Server {
    /// Packet server handle.
    net: Network,
    /// Clients of this server, these structures track the network state of each client.
    clients: HashMap<NetworkClient, ClientState>,
    /// Worlds list.
    worlds: Vec<WorldState>,
    /// Offline players database.
    offline_players: HashMap<String, OfflinePlayer>,
}

impl Server {
    /// Bind this server's TCP listener to the given address.
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        info!("server bound to {addr}");

        Ok(Self {
            net: Network::bind(addr)?,
            clients: HashMap::new(),
            worlds: vec![],
            offline_players: HashMap::new(),
        })
    }

    /// Register a world in this server.
    pub fn register_world(&mut self, name: String, dimension: Dimension) {
        self.worlds.push(WorldState {
            world: ServerWorld::new(name, dimension),
            players: Vec::new(),
        });
    }

    /// Force save this server and block waiting for all resources to be saved.
    pub fn stop(&mut self) {
        for state in &mut self.worlds {
            state.world.stop();
        }
    }

    /// Run a single tick on the server network and worlds. This function also waits for
    /// this function to approximately last for 50 ms (20 TPS), there is no sleep of the
    /// tick was too long, in such case a warning is logged.
    pub fn tick_padded(&mut self) -> io::Result<()> {
        let start = Instant::now();
        self.tick()?;
        let elapsed = start.elapsed();

        if let Some(missing) = TICK_DURATION.checked_sub(elapsed) {
            std::thread::sleep(missing);
        } else {
            warn!("tick too long {:?}, expected {:?}", elapsed, TICK_DURATION);
        }

        Ok(())
    }

    /// Run a single tick on the server network and worlds.
    pub fn tick(&mut self) -> io::Result<()> {
        // Start by ticking the network, we receive and process all packets from clients.
        // All client-world interactions happens here.
        self.tick_net()?;

        // Then we tick each world.
        for state in &mut self.worlds {
            state.world.tick(&mut state.players);
        }

        Ok(())
    }

    /// Tick the network and accept incoming events.
    fn tick_net(&mut self) -> io::Result<()> {
        // Poll all network events.
        while let Some(event) = self.net.poll()? {
            match event {
                NetworkEvent::Accept { client } => self.handle_accept(client),
                NetworkEvent::Lost { client, error } => self.handle_lost(client, error),
                NetworkEvent::Packet { client, packet } => self.handle_packet(client, packet),
            }
        }

        Ok(())
    }

    /// Handle new client accepted by the network.
    fn handle_accept(&mut self, client: NetworkClient) {
        info!("accept client #{}", client.id());
        self.clients.insert(client, ClientState::Handshaking);
    }

    /// Handle a lost client.
    fn handle_lost(&mut self, client: NetworkClient, error: Option<io::Error>) {
        info!("lost client #{}: {:?}", client.id(), error);

        let state = self.clients.remove(&client).unwrap();

        if let ClientState::Playing {
            world_index,
            player_index,
        } = state
        {
            // Save current player state
            self.save_player_state(world_index, player_index);
            // If the client was playing, remove it from its world.
            let state = &mut self.worlds[world_index];
            // Swap remove the player and tell the world.
            let mut player = state.players.swap_remove(player_index);
            state.world.handle_player_leave(&mut player, true);
            // If a player has been swapped in place of this new one, redefine its state.
            if let Some(swapped_player) = state.players.get(player_index) {
                self.clients
                    .insert(
                        swapped_player.client,
                        ClientState::Playing {
                            world_index,
                            player_index,
                        },
                    )
                    .expect("swapped player should have a previous state");
            }
        }
    }

    fn handle_packet(&mut self, client: NetworkClient, packet: InPacket) {
        // println!("[{client:?}] Packet: {packet:?}");

        match *self.clients.get(&client).unwrap() {
            ClientState::Handshaking => {
                self.handle_handshaking(client, packet);
            }
            ClientState::Playing {
                world_index,
                player_index,
            } => {
                let state = &mut self.worlds[world_index];
                let player = &mut state.players[player_index];
                player.handle(&mut state.world, packet);
            }
        }
    }

    /// Handle a packet for a client that is in handshaking state.
    fn handle_handshaking(&mut self, client: NetworkClient, packet: InPacket) {
        match packet {
            InPacket::KeepAlive => {}
            InPacket::Handshake(_) => self.handle_handshake(client),
            InPacket::Login(packet) => self.handle_login(client, packet),
            _ => self.send_disconnect(client, format!("Invalid packet: {packet:?}")),
        }
    }

    /// Handle a handshake from a client that is still handshaking, there is no
    /// restriction.
    fn handle_handshake(&mut self, client: NetworkClient) {
        self.net.send(
            client,
            OutPacket::Handshake(proto::OutHandshakePacket {
                server: "-".to_string(),
            }),
        );
    }

    /// Handle a login after handshake.
    fn handle_login(&mut self, client: NetworkClient, packet: proto::InLoginPacket) {
        if packet.protocol_version != 14 {
            self.send_disconnect(client, "Protocol version mismatch!".to_string());
            return;
        }

        let spawn_pos = config::SPAWN_POS;

        // Get the offline player, if not existing we create a new one with the
        let offline_player = self
            .offline_players
            .entry(packet.username.clone())
            .or_insert(OfflinePlayer::new(
                self.worlds[0].world.name.clone(),
                spawn_pos,
            ));

        let (world_index, _) = self
            .worlds
            .iter_mut()
            .enumerate()
            .find(|(_, state)| state.world.name == offline_player.world)
            .expect("invalid offline player world name");

        let entity = e::Human::new_with(|base, living, player| {
            base.pos = offline_player.pos;
            base.look = offline_player.look;
            base.persistent = false;
            base.can_pickup = true;
            living.artificial = true;
            living.health = 200; // FIXME: Lot of HP for testing.
            player.username = packet.username.clone();
        });

        let entity_id = self.worlds[world_index].world.world.spawn_entity(entity);
        self.worlds[world_index]
            .world
            .world
            .set_player_entity(entity_id, true);

        // Confirm the login by sending same packet in response.
        self.net.send(
            client,
            OutPacket::Login(proto::OutLoginPacket {
                entity_id,
                random_seed: self.worlds[world_index].world.seed,
                dimension: match self.worlds[world_index].world.world.get_dimension() {
                    Dimension::Overworld => 0,
                    Dimension::Nether => -1,
                },
            }),
        );

        // The standard server sends the spawn position just after login response.
        self.net.send(
            client,
            OutPacket::SpawnPosition(proto::SpawnPositionPacket {
                pos: spawn_pos.as_ivec3(),
            }),
        );

        // Time must be sent once at login to conclude the login phase.
        self.net.send(
            client,
            OutPacket::UpdateTime(proto::UpdateTimePacket {
                time: self.worlds[world_index].world.world.get_time(),
            }),
        );

        if self.worlds[world_index].world.world.get_weather() != Weather::Clear {
            self.net.send(
                client,
                OutPacket::Notification(proto::NotificationPacket { reason: 1 }),
            );
        }

        // Finally insert the player tracker.
        let mut player = ServerPlayer::new(
            &self.net,
            client,
            entity_id,
            packet.username,
            offline_player,
        );
        self.restore_player_state(client, &player);
        self.worlds[world_index]
            .world
            .handle_player_join(&mut player);
        let player_index = self.worlds[world_index].players.len();
        self.worlds[world_index].players.push(player);

        // Replace the previous state with a playing state containing the world and
        // player indices, used to get to the player instance.
        let previous_state = self.clients.insert(
            client,
            ClientState::Playing {
                world_index,
                player_index,
            },
        );

        // Just a sanity check...
        debug_assert_eq!(previous_state, Some(ClientState::Handshaking));
    }

    fn save_player_state(&mut self, world_index: usize, player_index: usize) {
        let state = &self.worlds[world_index];
        let player = &state.players[player_index];
        self.offline_players.insert(
            player.username.clone(),
            OfflinePlayer {
                world: state.world.name.clone(),
                pos: player.pos
                    + glam::DVec3 {
                        x: 0.0,
                        y: 1.72,
                        z: 0.0,
                    },
                look: player.look,
                main_inv: player.main_inv.to_owned(),
                armor_inv: player.armor_inv.to_owned(),
                craft_inv: player.craft_inv.to_owned(),
                cursor_stack: player.cursor_stack,
                hand_slot: player.hand_slot,
            },
        );
    }

    fn restore_player_state(&self, client: NetworkClient, player: &ServerPlayer) {
        // Send the initial position for the client.
        self.net.send(
            client,
            OutPacket::PositionLook(proto::PositionLookPacket {
                pos: player.pos,
                stance: player.pos.y + 1.62,
                look: player.look * (360.0 / core::f32::consts::TAU),
                on_ground: false,
            }),
        );
        // Send the initial crafting inventory for the client.
        for i in 0..4 {
            self.net.send(
                client,
                OutPacket::WindowSetItem(proto::WindowSetItemPacket {
                    window_id: 0,
                    slot: i + 1,
                    stack: player.craft_inv[i as usize].to_non_empty(),
                }),
            );
        }
        // Send the initial armor inventory for the client.
        for i in 0..4 {
            self.net.send(
                client,
                OutPacket::WindowSetItem(proto::WindowSetItemPacket {
                    window_id: 0,
                    slot: i + 5,
                    stack: player.armor_inv[i as usize].to_non_empty(),
                }),
            );
        }
        // Send the initial main inventory for the client.
        for i in 0..36 {
            self.net.send(
                client,
                OutPacket::WindowSetItem(proto::WindowSetItemPacket {
                    window_id: 0,
                    slot: i + 9,
                    stack: player.main_inv[((i + 9) % 36) as usize].to_non_empty(),
                }),
            );
        }
    }

    /// Send disconnect (a.k.a. kick) to a client.
    fn send_disconnect(&mut self, client: NetworkClient, reason: String) {
        if let Some(&ClientState::Playing {
            world_index,
            player_index,
        }) = self.clients.get(&client)
        {
            self.save_player_state(world_index, player_index);
        }
        self.net.send(
            client,
            OutPacket::Disconnect(proto::DisconnectPacket { reason }),
        )
    }
}

/// Track state of a network client in the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientState {
    /// This client is not yet connected to the world.
    Handshaking,
    /// This client is actually playing into a world.
    Playing {
        /// Index of the world this player is in.
        world_index: usize,
        /// Index of the player within the server world.
        player_index: usize,
    },
}

/// A server world registered in the server, it is associated to a list of players.
struct WorldState {
    /// The inner server world.
    world: ServerWorld,
    /// The players currently in this world.
    players: Vec<ServerPlayer>,
}
