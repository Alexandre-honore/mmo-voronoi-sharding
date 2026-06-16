// src/main.rs
mod voronoi;
mod render;
mod engine;
mod network;

use engine::{SimulatedGameServer, SpatialOrchestrator};
use macroquad::prelude::*;
use network::SimulationPacket;
use std::sync::mpsc;
use voronoi::Point2D;

fn window_conf() -> Conf {
    Conf {
        window_title: "MMO Orchestrator - Mirror of Production".parse().unwrap(),
        window_width: 1600,
        window_height: 1200,
        high_dpi: true,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let screen_w = screen_width();
    let screen_h = screen_height();

    // 1. Définition des canaux de communication réseau simulés
    let (tx_gs_to_sp, rx_gs_to_sp) = mpsc::channel::<SimulationPacket>();
    let (tx_sp_to_gs, rx_sp_to_gs) = mpsc::channel::<SimulationPacket>();

    // 2. Initialisation du Game Server
    let mut game_server = SimulatedGameServer::new(tx_gs_to_sp.clone());

    // 3. Initialisation de l'Orchestrateur avec les paramètres de ta prod
    let ghost_margin = 50.0;
    let hysteresis_time = 1.5;
    let hysteresis_distance = 15.0; // Marge spatiale pour le rayon de sécurité Voronoï

    let mut orchestrator = SpatialOrchestrator::new(
        Rect::new(0.0, 0.0, screen_w, screen_h),
        ghost_margin,
        hysteresis_time,
        rx_gs_to_sp,
        tx_sp_to_gs.clone()
    );

    // On spawn le serveur ROOT (id = 1) au centre de la map
    orchestrator.voronoi.spawn_shard(1, Point2D { x: screen_w * 0.5, y: screen_h * 0.5 }, 0);

    let voronoi_tick_rate = 0.5; // Topologie spatiale calculée à 2 Hz
    let mut time_since_last_tick = 0.0;
    let mut time_since_last_heartbeat = 0.0;

    loop {
        let dt = get_frame_time();
        orchestrator.sim_time += dt;
        clear_background(BLACK);

        // --- 1. ENTRÉES UTILISATEUR (Populer le monde) ---
        if is_mouse_button_pressed(MouseButton::Right) {
            let (mx, my) = mouse_position();
            let angle = macroquad::rand::gen_range(0.0, std::f32::consts::PI * 2.0);
            game_server.spawn_player(mx, my, angle);
        }

        // --- 2. GAME SERVER : Physique et Heartbeats ---
        game_server.tick_physics(dt, screen_w, screen_h);

        time_since_last_heartbeat += dt;
        if time_since_last_heartbeat >= 1.0 {
            time_since_last_heartbeat = 0.0;
            // Le GameServer envoie ses stats MapReduce (count, etc.) à 1 Hz
            game_server.send_heartbeats();
        }

        // --- 3. ORCHESTRATEUR : Ingestion Continue ---
        // Lit les paquets UDP simulés reçus et met à jour les dictionnaires O(1)
        orchestrator.process_network_inbox();

        // --- 4. ORCHESTRATEUR : Le Global Tick (2 Hz) ---
        time_since_last_tick += dt;
        if time_since_last_tick >= voronoi_tick_rate {
            time_since_last_tick = 0.0;

            // A. Construction des métriques
            // Dans ton vrai jeu, tu utiliseras orchestrator.latest_occupancy pour les splits
            // Ici on utilise build_metrics() pour avoir les barycentres et voir la carte bouger
            let metrics = orchestrator.voronoi.build_metrics();

            // B. Mécanique des fluides (Relaxation de Lloyd vers les joueurs)
            orchestrator.voronoi.relax_shards(&metrics, 0.6);

            // C. Mise à jour de la topologie géométrique globale
            orchestrator.voronoi.update_voronoi_data(ghost_margin, hysteresis_distance);

            // [NOTE] : C'est ici que tu brancheras ta logique de :
            // if orchestrator.latest_occupancy.get(shard_id) > LIMIT { spawn_shard(...) }
        }

        // --- 5. GAME SERVER : Acceptation des Transferts ---
        while let Ok(msg) = rx_sp_to_gs.try_recv() {
            match msg {
                SimulationPacket::HandoffRequest { shard_id, client_id } => {
                    // Dans la simulation, le GS accepte les ghosts instantanément
                    let _ = tx_gs_to_sp.send(SimulationPacket::HandoffAccept { shard_id, client_id });
                }
                SimulationPacket::HandoffComplete { new_shard_id, client_id, .. } => {
                    // Le transfert d'autorité est validé
                    game_server.process_handoffs(client_id, new_shard_id);
                }
                _ => {}
            }
        }

        // --- 6. RENDU VISUEL ---
        // On dessine le diagramme en récupérant les données directes de VoronoiMap
        let shards_vec: Vec<_> = orchestrator.voronoi.shards.values().cloned().collect();
        crate::render::draw_voronoi_polygons(&shards_vec, &orchestrator.voronoi.cells);

        // On dessine les joueurs
        for &player_pos in orchestrator.voronoi.players.values() {
            draw_circle(player_pos.x, player_pos.y, 10.0, WHITE);
        }

        // UI de base
        draw_text(
            &format!("FPS: {} | Joueurs: {} | Serveurs: {}",
                     get_fps(), orchestrator.voronoi.players.len(), orchestrator.voronoi.shards.len()
            ),
            10.0, 20.0, 20.0, WHITE
        );
        draw_text("Clic Droit: Maintenir pour Spawn des Joueurs", 10.0, 45.0, 20.0, YELLOW);

        next_frame().await
    }
}