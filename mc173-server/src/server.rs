//! The network server managing connected players and dispatching incoming packets.

use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::io;

use glam::{Vec2, DVec3};

use log::{warn, info};

use mc173::world::{Dimension, Weather};
use mc173::entity::{self as e};

use crate::proto::{self, Network, NetworkEvent, NetworkClient, InPacket, OutPacket};
use crate::offline::OfflinePlayer;
use crate::player::ServerPlayer;
use crate::world::ServerWorld;


/// Target tick duration. Currently 20 TPS, so 50 ms/tick.
const TICK_DURATION: Duration = Duration::from_millis(50);


/// This structure manages a whole server and its clients, dispatching incoming packets
/// to correct handlers.
pub struct Server {
    /// Packet server handle.
    net: Network,
    /// Clients of this server, these structures track the network state of each client.
    clients: HashMap<NetworkClient, ClientState>,
    /// Worlds list.
    worlds: Vec<ServerWorld>,
    /// Offline players
    offline_players: HashMap<String, OfflinePlayer>,
}

impl Server {

    /// Bind this server's TCP listener to the given address.
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {

        info!("server bound to {addr}");

        Ok(Self {
            net: Network::bind(addr)?,
            clients: HashMap::new(),
            worlds: vec![
                ServerWorld::new("overworld"),
            ],
            offline_players: HashMap::new(),
        })

    }

    /// Force save this server and block waiting for all resources to be saved.
    pub fn save(&mut self) {

        for world in &mut self.worlds {
            world.save();
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
            warn!("tick take too long {:?}, expected {:?}", elapsed, TICK_DURATION);
        }

        Ok(())

    }

    /// Run a single tick on the server network and worlds.
    pub fn tick(&mut self) -> io::Result<()> {

        // Poll all network events.
        while let Some(event) = self.net.poll()? {
            match event {
                NetworkEvent::Accept { client } => 
                    self.handle_accept(client),
                NetworkEvent::Lost { client, error } => 
                    self.handle_lost(client, error),
                NetworkEvent::Packet { client, packet } => 
                    self.handle_packet(client, packet),
            }
        }

        for world in &mut self.worlds {
            world.tick();
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
        
        if let ClientState::Playing { world_index, player_index } = state {
            // If the client was playing, remove it from its world.
            let world = &mut self.worlds[world_index];
            if let Some(swapped_player) = world.handle_player_leave(player_index, true) {
                // If a player has been swapped in place of the removed one, update the 
                // swapped one to point to its new index (and same world).
                let state = self.clients.get_mut(&swapped_player.client)
                    .expect("swapped player should be existing");
                *state = ClientState::Playing { world_index, player_index };
            }
        }

    }

    fn handle_packet(&mut self, client: NetworkClient, packet: InPacket) {
        
        // println!("[{client:?}] Packet: {packet:?}");

        match *self.clients.get(&client).unwrap() {
            ClientState::Handshaking => {
                self.handle_handshaking(client, packet);
            }
            ClientState::Playing { world_index, player_index } => {
                let world = &mut self.worlds[world_index];
                let player = &mut world.players[player_index];
                player.handle(&mut world.world, &mut world.state, packet);
            }
        }

    }

    /// Handle a packet for a client that is in handshaking state.
    fn handle_handshaking(&mut self, client: NetworkClient, packet: InPacket) {
        match packet {
            InPacket::KeepAlive => {}
            InPacket::Handshake(_) => 
                self.handle_handshake(client),
            InPacket::Login(packet) =>
                self.handle_login(client, packet),
            _ => self.send_disconnect(client, format!("Invalid packet: {packet:?}"))
        }
    }

    /// Handle a handshake from a client that is still handshaking, there is no 
    /// restriction.
    fn handle_handshake(&mut self, client: NetworkClient) {
        self.net.send(client, OutPacket::Handshake(proto::OutHandshakePacket {
            server: "-".to_string(),
        }));
    }

    /// Handle a login after handshake.
    fn handle_login(&mut self, client: NetworkClient, packet: proto::InLoginPacket) {

        if packet.protocol_version != 14 {
            self.send_disconnect(client, format!("Protocol version mismatch!"));
            return;
        }

        let spawn_pos = DVec3::new(0.0, 100.0, 0.0);

        // Get the offline player, if not existing we create a new one with the 
        let offline_player = self.offline_players.entry(packet.username.clone())
            .or_insert_with(|| {
                let spawn_world = &self.worlds[0];
                OfflinePlayer {
                    world: spawn_world.state.name.clone(),
                    pos: spawn_pos,
                    look: Vec2::ZERO,
                }
            });

        let (world_index, world) = self.worlds.iter_mut()
            .enumerate()
            .filter(|(_, world)| world.state.name == offline_player.world)
            .next()
            .expect("invalid offline player world name");

        let entity = e::Player::new_with(|base, _, player| {
            player.username = packet.username.clone();
            base.pos = offline_player.pos;
            base.look = offline_player.look;
            base.persistent = false;
            base.can_pickup = true;
            base.controlled = true;
            base.health = 20;
        });

        let entity_id = world.world.spawn_entity(entity);

        // Confirm the login by sending same packet in response.
        self.net.send(client, OutPacket::Login(proto::OutLoginPacket {
            entity_id,
            random_seed: world.state.seed,
            dimension: match world.world.get_dimension() {
                Dimension::Overworld => 0,
                Dimension::Nether => -1,
            },
        }));

        // The standard server sends the spawn position just after login response.
        self.net.send(client, OutPacket::SpawnPosition(proto::SpawnPositionPacket {
            pos: spawn_pos.as_ivec3(),
        }));

        // Send the initial position for the client.
        self.net.send(client, OutPacket::PositionLook(proto::PositionLookPacket {
            pos: offline_player.pos,
            stance: offline_player.pos.y + 1.62,
            look: offline_player.look,
            on_ground: false,
        }));

        // Time must be sent once at login to conclude the login phase.
        self.net.send(client, OutPacket::UpdateTime(proto::UpdateTimePacket {
            time: world.world.get_time(),
        }));

        if world.world.get_weather() != Weather::Clear {
            self.net.send(client, OutPacket::Notification(proto::NotificationPacket {
                reason: 1,
            }));
        }

        // Finally insert the player tracker.
        let server_player = ServerPlayer::new(&self.net, client, entity_id, packet.username, &offline_player);
        let player_index = world.handle_player_join(server_player);

        // Replace the previous state with a playing state containing the world and 
        // player indices, used to get to the player instance.
        let previous_state = self.clients.insert(client, ClientState::Playing {
            world_index,
            player_index,
        });

        // Just a sanity check...
        debug_assert_eq!(previous_state, Some(ClientState::Handshaking));

        // TODO: Broadcast chat joining chat message.

    }

    /// Send disconnect (a.k.a. kick) to a client.
    fn send_disconnect(&mut self, client: NetworkClient, reason: String) {
        self.net.send(client, OutPacket::Disconnect(proto::DisconnectPacket {
            reason,
        }))
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
    }
}
