// src/engine.rs
use crate::network::SimulationPacket;
use crate::voronoi::{Point2D, VoronoiMap};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::mpsc::{Receiver, Sender};

// ==========================================
// L'ORCHESTRATEUR SPATIAL (Miroir de ta Prod)
// ==========================================
pub struct SpatialOrchestrator {
    pub voronoi: VoronoiMap,

    // TON ETAT DE PRODUCTION EXACT
    pub client_to_shards: FxHashMap<u32, (u32, f32)>, // (shard_id, last_change_time en sec)
    pub ghost_client: FxHashMap<u32, Vec<u32>>,
    pub client_waiting_for_crossing: FxHashMap<u32, (u32, u32)>,

    pub margin: f32,
    pub hysteresis_time: f32,
    pub sim_time: f32,

    // Pour l'algorithme Voronoï
    pub latest_occupancy: FxHashMap<u32, u32>,
    pub next_shard_id: u32,

    rx_from_gs: Receiver<SimulationPacket>,
    tx_to_gs: Sender<SimulationPacket>,
}

impl SpatialOrchestrator {
    pub fn new(bounds: macroquad::prelude::Rect, margin: f32, hysteresis_time: f32, rx_from_gs: Receiver<SimulationPacket>, tx_to_gs: Sender<SimulationPacket>) -> Self {
        Self {
            voronoi: VoronoiMap::new(bounds),
            client_to_shards: FxHashMap::default(),
            ghost_client: FxHashMap::default(),
            client_waiting_for_crossing: FxHashMap::default(),
            margin,
            hysteresis_time,
            sim_time: 0.0,
            latest_occupancy: FxHashMap::default(),
            next_shard_id: 2,
            rx_from_gs,
            tx_to_gs,
        }
    }

    pub fn process_network_inbox(&mut self) {
        while let Ok(msg) = self.rx_from_gs.try_recv() {
            match msg {
                SimulationPacket::PlayerJoinUpdate { client_id, x, y } => {
                    let pos = Point2D { x, y };
                    if let Some(shard_id) = self.voronoi.insert_player(client_id, pos) {
                        self.client_to_shards.insert(client_id, (shard_id, self.sim_time));
                        let _ = self.tx_to_gs.send(SimulationPacket::SpawnPlayerShard { shard_id, client_id });
                    }
                }
                SimulationPacket::PositionUpdate { client_id, x, y } => {
                    self.process_position_update(client_id, Point2D { x, y });
                }
                SimulationPacket::ClientLeft { client_id } => {
                    if let Some(&(shard_id, _)) = self.client_to_shards.get(&client_id) {
                        self.voronoi.remove_player(client_id, shard_id);
                        self.client_to_shards.remove(&client_id);
                        let _ = self.tx_to_gs.send(SimulationPacket::DespawnPlayerShard { shard_id, client_id });
                    }
                }
                SimulationPacket::ServerHeartBeat { shard_id, occupancy } => {
                    self.latest_occupancy.insert(shard_id, occupancy);
                }
                SimulationPacket::HandoffAccept { shard_id, client_id } => {
                    self.process_handoff_accept(shard_id, client_id);
                }
                _ => {}
            }
        }
    }

    pub fn process_position_update(&mut self, client_id: u32, new_pos: Point2D) {
        let Some(&(old_shard_id, last_time)) = self.client_to_shards.get(&client_id) else { return; };
        let Some(new_shard_id) = self.voronoi.shard_id_for(new_pos) else { return; };

        let old_pos = self.voronoi.hard_update_player_position(client_id, old_shard_id, new_pos).unwrap_or(new_pos);

        // HYSTERESIS CROSSING
        if old_shard_id != new_shard_id && (self.sim_time - last_time) > self.hysteresis_time {
            self.apply_client_cross(client_id, old_shard_id, new_shard_id, new_pos);
        }

        // GHOSTING (SUBSCRIBE / UNSUBSCRIBE) basés sur la marge Voronoï
        let current_visible = self.voronoi.shards_near(new_pos, self.margin);
        let past_visible = self.voronoi.shards_near(old_pos, self.margin);

        let current_set: FxHashSet<_> = current_visible.into_iter().collect();
        let past_set: FxHashSet<_> = past_visible.into_iter().collect();

        // Unsubscribe
        for &left_shard in past_set.difference(&current_set) {
            let _ = self.tx_to_gs.send(SimulationPacket::HandoffDrop { shard_id: left_shard, client_id });
            if let Some(ghosts) = self.ghost_client.get_mut(&left_shard) { ghosts.retain(|&id| id != client_id); }
        }

        // Subscribe
        for &entered_shard in current_set.difference(&past_set) {
            let _ = self.tx_to_gs.send(SimulationPacket::HandoffRequest { shard_id: entered_shard, client_id });
        }
    }

    pub fn apply_client_cross(&mut self, client_id: u32, old_shard_id: u32, new_shard_id: u32, _pos: Point2D) {
        self.client_waiting_for_crossing.remove(&client_id);

        if let Some(ghosts) = self.ghost_client.get_mut(&new_shard_id) {
            if ghosts.contains(&client_id) {
                ghosts.retain(|&id| id != client_id);
                self.ghost_client.entry(old_shard_id).or_default().push(client_id);
                let _ = self.tx_to_gs.send(SimulationPacket::HandoffComplete { new_shard_id, old_shard_id, client_id });
            } else {
                self.client_waiting_for_crossing.insert(client_id, (old_shard_id, new_shard_id));
            }
        } else {
            self.client_waiting_for_crossing.insert(client_id, (old_shard_id, new_shard_id));
        }
        self.client_to_shards.insert(client_id, (new_shard_id, self.sim_time));
    }

    pub fn process_handoff_accept(&mut self, shard_id: u32, client_id: u32) {
        let Some(&(old_shard_id, _)) = self.client_to_shards.get(&client_id) else { return; };

        if let Some(&(wait_old, wait_new)) = self.client_waiting_for_crossing.get(&client_id) {
            if wait_new == shard_id && wait_old == old_shard_id {
                self.client_waiting_for_crossing.remove(&client_id);
                let _ = self.tx_to_gs.send(SimulationPacket::HandoffComplete { new_shard_id: shard_id, old_shard_id, client_id });
                return;
            }
        }
        self.ghost_client.entry(shard_id).or_default().push(client_id);
    }
}

// ==========================================
// LE GAME SERVER SIMULÉ
// ==========================================
pub struct SimulatedGameServer {
    pub players_sim: Vec<(u32, Point2D, Point2D, u32)>, // (net_id, pos, vel, current_shard_id)
    tx_to_spatial: Sender<SimulationPacket>,
    pub next_net_id: u32,
}

impl SimulatedGameServer {
    pub fn new(tx_to_spatial: Sender<SimulationPacket>) -> Self {
        Self { players_sim: Vec::new(), tx_to_spatial, next_net_id: 101 }
    }

    pub fn spawn_player(&mut self, x: f32, y: f32, angle: f32) {
        let client_id = self.next_net_id;
        self.next_net_id += 1;
        self.players_sim.push((client_id, Point2D { x, y }, Point2D { x: angle.cos(), y: angle.sin() }, 1));
        let _ = self.tx_to_spatial.send(SimulationPacket::PlayerJoinUpdate { client_id, x, y });
    }

    pub fn tick_physics(&mut self, dt: f32, bounds_w: f32, bounds_h: f32) {
        let speed = 150.0;
        for (client_id, pos, vel, _) in &mut self.players_sim {
            pos.x += vel.x * speed * dt;
            pos.y += vel.y * speed * dt;
            if pos.x < 15.0 || pos.x > bounds_w - 15.0 { vel.x = -vel.x; pos.x = pos.x.clamp(15.0, bounds_w - 15.0); }
            if pos.y < 15.0 || pos.y > bounds_h - 15.0 { vel.y = -vel.y; pos.y = pos.y.clamp(15.0, bounds_h - 15.0); }

            // Jittering
            if macroquad::rand::gen_range(0, 60) == 0 {
                let _ = self.tx_to_spatial.send(SimulationPacket::PositionUpdate { client_id: *client_id, x: pos.x, y: pos.y });
            }
        }
    }

    pub fn send_heartbeats(&self) {
        let mut occupancy: FxHashMap<u32, u32> = FxHashMap::default();
        for (_, _, _, shard_id) in &self.players_sim {
            *occupancy.entry(*shard_id).or_insert(0) += 1;
        }
        for (shard_id, count) in occupancy {
            let _ = self.tx_to_spatial.send(SimulationPacket::ServerHeartBeat { shard_id, occupancy: count });
        }
    }

    pub fn process_handoffs(&mut self, client_id: u32, new_shard: u32) {
        if let Some(p) = self.players_sim.iter_mut().find(|p| p.0 == client_id) {
            p.3 = new_shard;
        }
    }
}