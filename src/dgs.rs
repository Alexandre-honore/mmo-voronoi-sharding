use slotmap::{DenseSlotMap, SlotMap};
// DGS de simulation, un seul gère tous les joueurs
use rustc_hash::FxHashMap;
use crate::shared::{Point2D, PlayerKey, ShardKey, Player, Shard};
use crate::spatial::SpatialService;

pub struct DigitalGameServer{
    players: DenseSlotMap<PlayerKey, Player>,
    current_tick: u64,
    telemetry_timer: f32,
    telemetry_interval: f32,
}

impl Default for DigitalGameServer {
    fn default() -> Self {
        Self::new()
    }
}

impl DigitalGameServer {
    pub fn new() -> Self {
        Self {
            players: DenseSlotMap::with_key(),
            current_tick: 0,
            telemetry_timer: 0.0,
            telemetry_interval: 1.0, // Rapport d'occupation envoyé toutes les 1.0 secondes
        }
    }

    //////////////////////////
    //    API SIMULATION    //
    //////////////////////////
    pub fn add_player(&mut self, pos: Point2D, initial_shard: ShardKey, spatial: &mut SpatialService) -> PlayerKey {
        let key = self.players.insert(Player { pos, current_shard: initial_shard, ghost_shards: Vec::new() });
        println!("[DGS] Création du joueur {:?} avec l'autorité. Transmission de la position au SpatialService...", key);
        spatial.add_player(key, pos, initial_shard);
        key
    }

    pub fn get_player_at_location(&self, location: &Point2D, radius: f32) -> Option<PlayerKey> {
        let radius_sq = radius * radius;
        for (key, player) in self.players.iter() {
            if player.pos.distance_sq(location) <= radius_sq {
                return Some(key);
            }
        }
        None
    }

    // Le DGS met à jour sa référence autoritaire, puis synchronise le Spatial
    pub fn move_player(&mut self, key: PlayerKey, new_pos: Point2D, spatial: &mut SpatialService) {
        if let Some(player) = self.players.get_mut(key) {
            player.pos = new_pos;
            spatial.update_player_position(key, new_pos);
        }
    }


    //////////////////////////

    fn report_occupancy(&self, spatial: &mut SpatialService) {
        let mut counts: FxHashMap<ShardKey, u32> = FxHashMap::default();

        // On compte les joueurs par zone (Dans un vrai jeu, tu pourrais ajouter la CPU load ici)
        for (_, player) in self.players.iter() {
            *counts.entry(player.current_shard).or_insert(0) += 1;
        }

        let mut occupancies: FxHashMap<ShardKey, f32> = FxHashMap::default();

        // Capacité maximale simulée pour conserver ton ancien comportement (5 joueurs = 100%)
        let max_simulated_capacity = 5.0;

        for (key, count) in counts {
            let occupancy_percent = (count as f32) / max_simulated_capacity;
            occupancies.insert(key, occupancy_percent);
        }

        println!("[DGS] Envoi de la télémétrie réseau. Mise à jour de l'occupation des Shards.");
        spatial.update_shard_occupancies(occupancies);
    }

    pub fn tick(&mut self, dt: f32, spatial: &mut SpatialService, voronoi_updated: bool) {
        self.current_tick += 1;
        self.telemetry_timer += dt;

        // 1. Télémétrie : Envoi de l'occupation au SpatialService
        if self.telemetry_timer >= self.telemetry_interval {
            self.telemetry_timer -= self.telemetry_interval;
            self.report_occupancy(spatial);
        }

        if voronoi_updated {
            // 1. Gestion des Handoffs (Autorité) - INCHANGÉ
            let handoffs = spatial.compute_pending_handoffs();
            for (player_key, old_shard, new_shard) in handoffs {
                if let Some(player) = self.players.get_mut(player_key) {
                    player.current_shard = new_shard;
                    println!("[DGS] HANDOFF VALIDÉ : Joueur {:?} (Shard {:?} -> {:?})", player_key, old_shard, new_shard);
                    spatial.update_player_shard(player_key, new_shard);
                }
            }

            // 2. NOUVEAU : Gestion de l'Interest Management (Ghosts)
            let ghost_map = spatial.compute_ghost_visibility();

            for (player_key, player) in self.players.iter_mut() {
                let new_ghosts = ghost_map.get(&player_key).cloned().unwrap_or_default();

                // On ne log que si la situation a changé pour éviter le spam
                if player.ghost_shards != new_ghosts {
                    if !new_ghosts.is_empty() {
                        println!("[DGS] Joueur {:?} entre dans les ghost zones répliquées : {:?}", player_key, new_ghosts);
                    } else {
                        println!("[DGS] Joueur {:?} quitte les ghost zones.", player_key);
                    }
                    player.ghost_shards = new_ghosts;
                }
            }
        }
    }
}