// src/main.rs
mod voronoi;
mod quadtree;
mod render;

use macroquad::prelude::*;
use std::collections::HashMap;
use voronoi::*;
use quadtree::*;
use render::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "MMO Network Engine - Handoff & PubSub Debugger".parse().unwrap(),
        window_width: 1600,
        window_height: 1200,
        high_dpi: true,
        ..Default::default()
    }
}

#[derive(PartialEq)]
enum AppMode {
    Menu,
    Voronoi,
    Quadtree,
}

#[macroquad::main(window_conf)]
async fn main() {
    next_frame().await;

    let screen_w = screen_width();
    let screen_h = screen_height();

    let mut mode = AppMode::Menu;
    let mut current_tick: u64 = 0;
    let mut next_shard_id = 2;

    // --- ÉTAT VORONOI ---
    let mut voronoi_shards = vec![
        Shard { id: 1, pos: Point2D { x: screen_w * 0.5, y: screen_h * 0.5 }, spawn_tick: 0 },
    ];
    let mut voronoi_data: HashMap<u32, VoronoiCellData> = HashMap::new();

    // --- ÉTAT QUADTREE ---
    let mut quad_root = QuadNode::new(1, QuadRect { x: 0.0, y: 0.0, w: screen_w, h: screen_h }, 0);

    // --- ÉTAT PARTAGÉ ---
    let mut players: Vec<Player> = Vec::new();
    let mut next_player_id = 101;
    let mut selected_player_idx: Option<usize> = None;
    let hysteresis_margin = 30.0;
    let mut auto_move = false;
    let mut player_angles: HashMap<u32, f32> = HashMap::new();

    // --- STATISTIQUES ET RESEAU ---
    let mut sim_time: f32 = 0.0;
    let mut stats_handoffs: u32 = 0;
    let mut stats_splits: u32 = 0;
    let mut stats_merges: u32 = 0;

    let mut show_network = false;
    let chunk_size = 100.0_f32;
    let ghost_margin = 50.0_f32;

    loop {
        current_tick += 1;
        let (mx, my) = mouse_position();
        let mouse_pos = Point2D { x: mx, y: my };

        clear_background(BLACK);

        // ==============================
        //           MENU MODE
        // ==============================
        if mode == AppMode::Menu {
            let cx = screen_w / 2.0;
            let cy = screen_h / 2.0;

            draw_text("CHOISISSEZ L'ARCHITECTURE DE SHARDING", cx - 250.0, cy - 100.0, 30.0, WHITE);

            let btn_v_rect = Rect { x: cx - 350.0, y: cy, w: 300.0, h: 100.0 };
            let btn_q_rect = Rect { x: cx + 50.0, y: cy, w: 300.0, h: 100.0 };

            draw_rectangle(btn_v_rect.x, btn_v_rect.y, btn_v_rect.w, btn_v_rect.h, DARKGRAY);
            draw_text("VORONOI (Data-Oriented)", btn_v_rect.x + 10.0, btn_v_rect.y + 60.0, 25.0, WHITE);

            draw_rectangle(btn_q_rect.x, btn_q_rect.y, btn_q_rect.w, btn_q_rect.h, DARKGRAY);
            draw_text("QUADTREE (Grille Fixe)", btn_q_rect.x + 20.0, btn_q_rect.y + 60.0, 25.0, WHITE);

            if is_mouse_button_pressed(MouseButton::Left) {
                let mouse_vec = Vec2::new(mouse_pos.x, mouse_pos.y);
                if btn_v_rect.contains(mouse_vec) { mode = AppMode::Voronoi; }
                if btn_q_rect.contains(mouse_vec) { mode = AppMode::Quadtree; }
            }

            next_frame().await;
            continue;
        }

        // ==============================
        //      LOGIQUE PARTAGÉE
        // ==============================
        if is_key_pressed(KeyCode::Space) { auto_move = !auto_move; }
        if is_key_pressed(KeyCode::N) { show_network = !show_network; }
        if is_key_pressed(KeyCode::Escape) {
            mode = AppMode::Menu;
            players.clear();
            sim_time = 0.0;
            stats_handoffs = 0; stats_splits = 0; stats_merges = 0;
            auto_move = false;
        }

        if is_mouse_button_pressed(MouseButton::Left) {
            for (i, player) in players.iter().enumerate() {
                if mouse_pos.distance_sq(&player.pos) < 15.0 * 15.0 {
                    selected_player_idx = Some(i);
                    break;
                }
            }
        }
        if is_mouse_button_released(MouseButton::Left) { selected_player_idx = None; }

        if is_mouse_button_pressed(MouseButton::Right) {
            let initial_shard_id = if mode == AppMode::Voronoi {
                find_nearest_shard_id(&mouse_pos, &voronoi_shards)
            } else {
                quad_root.find_leaf_id(&mouse_pos)
            };

            players.push(Player {
                id: next_player_id, pos: mouse_pos, current_shard_id: initial_shard_id,
            });
            next_player_id += 1;
        }

        if let Some(idx) = selected_player_idx { players[idx].pos = mouse_pos; }

        if auto_move {
            let dt = get_frame_time();
            sim_time += dt;
            let speed_per_sec = 150.0;

            for (i, player) in players.iter_mut().enumerate() {
                if Some(i) == selected_player_idx { continue; }
                let angle = player_angles.entry(player.id).or_insert_with(|| macroquad::rand::gen_range(0.0, std::f32::consts::PI * 2.0));
                *angle += macroquad::rand::gen_range(-2.0, 2.0) * dt;

                player.pos.x += angle.cos() * speed_per_sec * dt;
                player.pos.y += angle.sin() * speed_per_sec * dt;

                if player.pos.x < 15.0 || player.pos.x > screen_w - 15.0 {
                    *angle = std::f32::consts::PI - *angle;
                    player.pos.x = player.pos.x.clamp(15.0, screen_w - 15.0);
                }
                if player.pos.y < 15.0 || player.pos.y > screen_h - 15.0 {
                    *angle = -*angle;
                    player.pos.y = player.pos.y.clamp(15.0, screen_h - 15.0);
                }
            }
        }

        // ==============================
        //      MODE VORONOI (OPTIMISÉ)
        // ==============================
        if mode == AppMode::Voronoi {
            let id_before_split = next_shard_id;
            update_dynamic_sharding(&mut voronoi_shards, &mut players, &mut next_shard_id, current_tick);
            stats_splits += (next_shard_id - id_before_split) / 2;

            let id_before_merge = next_shard_id;
            merge_underpopulated_shards(&mut voronoi_shards, &players, &mut next_shard_id, current_tick);
            stats_merges += next_shard_id - id_before_merge;

            relax_shards(&mut voronoi_shards, &players, 0.05);

            // BACKEND : Calcul de la géométrie (Réseau + Polygones)
            voronoi_data = calculate_voronoi_data(&voronoi_shards, screen_w, screen_h, ghost_margin);

            // FRONTEND : Rendu GPU ultra-rapide
            draw_voronoi_polygons(&voronoi_shards, &voronoi_data);

            // Logique de Handoff
            for player in players.iter_mut() {
                let old_shard = player.current_shard_id;
                player.current_shard_id = evaluate_handoff(player, &voronoi_shards, hysteresis_margin);
                if player.current_shard_id != old_shard { stats_handoffs += 1; }
            }

            // Dessin des centres de serveurs
            for shard in voronoi_shards.iter() {
                let color = get_shard_color(shard.id);
                draw_circle(shard.pos.x, shard.pos.y, 8.0, color);
                draw_circle_lines(shard.pos.x, shard.pos.y, 10.0, 2.0, WHITE);
            }
        }

        // ==============================
        //      MODE QUADTREE
        // ==============================
        if mode == AppMode::Quadtree {
            let mut old_leaves = Vec::new();
            quad_root.collect_leaves(&mut old_leaves);
            let old_leaves_count = old_leaves.len() as i32;
            let id_before = next_shard_id;

            quad_root.update(&players, &mut next_shard_id, current_tick);

            let mut new_leaves = Vec::new();
            quad_root.collect_leaves(&mut new_leaves);
            let new_leaves_count = new_leaves.len() as i32;

            let splits = (next_shard_id - id_before) / 4;
            let delta_leaves = new_leaves_count - old_leaves_count;
            let merges = splits as i32 - (delta_leaves / 3);

            stats_splits += splits;
            stats_merges += merges.max(0) as u32;

            for player in players.iter_mut() {
                let old_shard = player.current_shard_id;
                player.current_shard_id = quad_root.find_leaf_id(&player.pos);
                if player.current_shard_id != old_shard { stats_handoffs += 1; }
            }

            for leaf in new_leaves {
                let mut color = get_shard_color(leaf.id);
                color.a = 0.3;
                draw_rectangle(leaf.rect.x, leaf.rect.y, leaf.rect.w, leaf.rect.h, color);
                draw_rectangle_lines(leaf.rect.x, leaf.rect.y, leaf.rect.w, leaf.rect.h, 2.0, WHITE);
                let cx = leaf.rect.x + leaf.rect.w / 2.0;
                let cy = leaf.rect.y + leaf.rect.h / 2.0;
                color.a = 1.0;
                draw_circle(cx, cy, 6.0, color);
            }
        }

        // ==============================
        //  RENDU DEBUG RÉSEAU (AABB)
        // ==============================
        if show_network && mode == AppMode::Voronoi {
            // Grille Pub/Sub de fond
            for x in (0..screen_w as u32).step_by(chunk_size as usize) {
                draw_line(x as f32, 0.0, x as f32, screen_h, 1.0, Color::new(1.0, 1.0, 1.0, 0.10));
            }
            for y in (0..screen_h as u32).step_by(chunk_size as usize) {
                draw_line(0.0, y as f32, screen_w, y as f32, 1.0, Color::new(1.0, 1.0, 1.0, 0.10));
            }

            for shard in voronoi_shards.iter() {
                if let Some(cell) = voronoi_data.get(&shard.id) {
                    let mut color = get_shard_color(shard.id);
                    let aabb = &cell.aabb;

                    // L'AABB du réseau
                    color.a = 0.15;
                    draw_rectangle(aabb.x, aabb.y, aabb.w, aabb.h, color);

                    color.a = 0.8;
                    draw_rectangle_lines(aabb.x, aabb.y, aabb.w, aabb.h, 2.0, color);

                    // Détection des Ghost Players
                    for player in players.iter() {
                        if player.current_shard_id != shard.id {
                            let player_vec = Vec2::new(player.pos.x, player.pos.y);
                            if aabb.contains(player_vec) {
                                draw_line(player.pos.x, player.pos.y, shard.pos.x, shard.pos.y, 1.0, color);
                                draw_circle_lines(player.pos.x, player.pos.y, 22.0, 2.0, color);
                            }
                        }
                    }
                }
            }
        }

        // --- RENDU COMMUN : LES JOUEURS ---
        for (i, player) in players.iter().enumerate() {
            let is_selected = selected_player_idx == Some(i);
            let owner_color = get_shard_color(player.current_shard_id);
            let fill_color = if is_selected { YELLOW } else { WHITE };

            draw_circle(player.pos.x, player.pos.y, 15.0, fill_color);
            draw_circle_lines(player.pos.x, player.pos.y, 15.0, 4.0, owner_color);
        }

        // --- UI ---
        let mode_str = if mode == AppMode::Voronoi { "VORONOI (Spade GPU)" } else { "QUADTREE" };
        let active_servers = if mode == AppMode::Voronoi { voronoi_shards.len() } else {
            let mut l = Vec::new(); quad_root.collect_leaves(&mut l); l.len()
        };
        draw_text(&format!("Mode: {} | FPS: {} | Joueurs: {} | Serveurs: {}", mode_str, get_fps(), players.len(), active_servers), 10.0, 20.0, 20.0, WHITE);
        draw_text("Espace: Auto-Move | N: Afficher AABB Réseau | Echap: Menu", 10.0, 45.0, 20.0, YELLOW);

        let mins = (sim_time / 60.0).floor() as u32;
        let secs = (sim_time % 60.0) as u32;

        draw_rectangle(10.0, 70.0, 350.0, 150.0, Color::new(0.0, 0.0, 0.0, 0.8));
        draw_rectangle_lines(10.0, 70.0, 350.0, 150.0, 2.0, GRAY);
        draw_text(&format!("Chrono : {:02}:{:02}", mins, secs), 20.0, 105.0, 25.0, WHITE);
        draw_text(&format!("Transferts : {}", stats_handoffs), 20.0, 140.0, 20.0, ORANGE);
        draw_text(&format!("Éclatements : {}", stats_splits), 20.0, 170.0, 20.0, RED);
        draw_text(&format!("Fusions : {}", stats_merges), 20.0, 200.0, 20.0, GREEN);

        next_frame().await
    }
}