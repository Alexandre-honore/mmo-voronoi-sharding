mod shared;
mod spatial;

mod dgs;
mod render;

use macroquad::prelude::*;
use crate::dgs::DigitalGameServer;
use crate::shared::{Point2D, PlayerKey};
use crate::spatial::SpatialService;

fn window_conf() -> Conf {
    Conf {
        window_title: "MMO Network Engine - Handoff & PubSub Debugger (ECS)".parse().unwrap(),
        window_width: 1600,
        window_height: 1200,
        high_dpi: true,
        ..Default::default()
    }
}

#[derive(PartialEq)]
enum AppMode {
    Menu, Voronoi
}

#[macroquad::main(window_conf)]
async fn main() {
    let screen_w = screen_width();
    let screen_h = screen_height();

    let mut mode = AppMode::Menu;
    let mut selected_player: Option<PlayerKey> = None;

    let mut spatial = SpatialService::new(5.0);
    spatial.update_map_size(screen_w, screen_h);
    let mut dgs = DigitalGameServer::new();
    let initial_shard = spatial.init_base_shards();

    loop {
        let (mx, my) = mouse_position();
        let mouse_pos = Point2D { x: mx, y: my };
        let dt = get_frame_time();

        clear_background(BLACK);

        if mode == AppMode::Menu {
            let cx = screen_w / 2.0; let cy = screen_h / 2.0;
            draw_text("CHOISISSEZ L'ARCHITECTURE DE SHARDING", cx - 250.0, cy - 100.0, 30.0, WHITE);

            let btn_v_rect = Rect { x: cx - 350.0, y: cy, w: 300.0, h: 100.0 };

            draw_rectangle(btn_v_rect.x, btn_v_rect.y, btn_v_rect.w, btn_v_rect.h, DARKGRAY);
            draw_text("VORONOI (Data-Oriented ECS)", btn_v_rect.x + 10.0, btn_v_rect.y + 60.0, 20.0, WHITE);

            if is_mouse_button_pressed(MouseButton::Left) {
                let mouse_vec = Vec2::new(mouse_pos.x, mouse_pos.y);
                if btn_v_rect.contains(mouse_vec) { mode = AppMode::Voronoi; }
            }
            next_frame().await;
            continue;
        }

        // ==============================
        //      MODE VORONOI (PUREMENT ECS)
        // ==============================
        if mode == AppMode::Voronoi {
            if is_mouse_button_pressed(MouseButton::Left) {
                // On utilise un rayon de 10 pixels (100 au carré) pour faciliter le clic
                if let Some(key) = dgs.get_player_at_location(&mouse_pos, 10.0) {
                    selected_player = Some(key);
                    println!("[MAIN] Joueur {:?} sélectionné pour déplacement.", key);
                }
            }

            // Clic Gauche (Relâche) : Désélection
            if is_mouse_button_released(MouseButton::Left) {
                if let Some(key) = selected_player {
                    println!("[MAIN] Joueur {:?} relâché.", key);
                }
                selected_player = None;
            }

            if let Some(entity) = selected_player {
                // TODO dire au spatial qu'on a bougé un player
            }

            if is_mouse_button_pressed(MouseButton::Right) {
                // On demande au DGS de faire spawn un joueur à la position de la souris
                println!("[MAIN] Clic droit détecté à la position ({:.2}, {:.2}). Demande de spawn au DGS...", mouse_pos.x, mouse_pos.y);
                let new_player_key = dgs.add_player(mouse_pos, initial_shard, &mut spatial);
            }

            if let Some(key) = selected_player {
                dgs.move_player(key, mouse_pos, &mut spatial);
            }

            let voronoi_updated = spatial.tick(dt);

            // Modification ici : on passe le dt
            dgs.tick(dt, &mut spatial, voronoi_updated);
        }

        // render
        let color_map = render::build_color_map(spatial.get_shards());

        // 2. On récupère les polygones
        let polygons = spatial.get_voronoi_polygons(screen_w, screen_h);

        // 3. On dessine dans le bon ordre (Fond -> Shards -> Joueurs)
        render::draw_voronoi(&polygons, &color_map);
        render::draw_shards(spatial.get_shards(), &color_map);
        render::draw_players(spatial.get_players(), &color_map);
        render::draw_ghost_aabbs(spatial.get_cells(), &color_map);

        // --- UI ---
        let mode_str = if mode == AppMode::Voronoi { "VORONOI (Data-Oriented ECS)" } else { "QUADTREE" };


        draw_text(&format!("Mode: {} | FPS: {}", mode_str, get_fps()), 10.0, 20.0, 20.0, WHITE);
        next_frame().await
    }
}