// src/main.rs
mod voronoi;
mod quadtree;
mod render;
mod engine;

use macroquad::prelude::*;
use std::collections::HashMap;
use voronoi::*;
use quadtree::*;
use render::*;
use engine::*;

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
    Menu, Voronoi, Quadtree,
}

#[macroquad::main(window_conf)]
async fn main() {
    let screen_w = screen_width();
    let screen_h = screen_height();

    let mut mode = AppMode::Menu;
    let mut sim = SimWorld::new(screen_w, screen_h);
    let mut selected_player: Option<hecs::Entity> = None;
    let mut quad_root = QuadNode::new(1, QuadRect { x: 0.0, y: 0.0, w: screen_w, h: screen_h }, 0);

    // Initial Shard pour le lancement
    sim.spawn_shard(screen_w * 0.5, screen_h * 0.5, 1, 0);

    let hysteresis_margin = 30.0;
    let mut auto_move = false;
    let mut show_network = false;
    let chunk_size = 100.0_f32;
    let ghost_margin = 50.0_f32;

    loop {
        sim.current_tick += 1;
        let (mx, my) = mouse_position();
        let mouse_pos = Point2D { x: mx, y: my };
        let dt = get_frame_time();

        clear_background(BLACK);

        if mode == AppMode::Menu {
            let cx = screen_w / 2.0; let cy = screen_h / 2.0;
            draw_text("CHOISISSEZ L'ARCHITECTURE DE SHARDING", cx - 250.0, cy - 100.0, 30.0, WHITE);

            let btn_v_rect = Rect { x: cx - 350.0, y: cy, w: 300.0, h: 100.0 };
            let btn_q_rect = Rect { x: cx + 50.0, y: cy, w: 300.0, h: 100.0 };

            draw_rectangle(btn_v_rect.x, btn_v_rect.y, btn_v_rect.w, btn_v_rect.h, DARKGRAY);
            draw_text("VORONOI (Data-Oriented ECS)", btn_v_rect.x + 10.0, btn_v_rect.y + 60.0, 20.0, WHITE);

            draw_rectangle(btn_q_rect.x, btn_q_rect.y, btn_q_rect.w, btn_q_rect.h, DARKGRAY);
            draw_text("QUADTREE (Grille Fixe)", btn_q_rect.x + 20.0, btn_q_rect.y + 60.0, 20.0, WHITE);

            if is_mouse_button_pressed(MouseButton::Left) {
                let mouse_vec = Vec2::new(mouse_pos.x, mouse_pos.y);
                if btn_v_rect.contains(mouse_vec) { mode = AppMode::Voronoi; }
                if btn_q_rect.contains(mouse_vec) { mode = AppMode::Quadtree; }
            }
            next_frame().await;
            continue;
        }

        // --- INPUTS ---
        if is_key_pressed(KeyCode::Space) { auto_move = !auto_move; }
        if is_key_pressed(KeyCode::N) { show_network = !show_network; }
        if is_key_pressed(KeyCode::Escape) {
            mode = AppMode::Menu;
            sim = SimWorld::new(screen_w, screen_h);
            sim.spawn_shard(screen_w * 0.5, screen_h * 0.5, 1, 0);
            selected_player = None;
            quad_root = QuadNode::new(1, QuadRect { x: 0.0, y: 0.0, w: screen_w, h: screen_h }, 0);
            auto_move = false;
        }

        if is_mouse_button_pressed(MouseButton::Left) {
            for (entity, (pos, _)) in sim.ecs.query::<(&Position, &PlayerInfo)>().iter() {
                if mouse_pos.distance_sq(&Point2D { x: pos.x, y: pos.y }) < 15.0 * 15.0 {
                    selected_player = Some(entity); break;
                }
            }
        }
        if is_mouse_button_released(MouseButton::Left) { selected_player = None; }

        if let Some(entity) = selected_player {
            if let Ok(mut pos) = sim.ecs.get::<&mut Position>(entity) { pos.x = mx; pos.y = my; }
        }

        if is_mouse_button_pressed(MouseButton::Right) {
            let initial_shard_id = if mode == AppMode::Voronoi {
                find_nearest_shard_id(&mouse_pos, &get_shards_vec(&sim.ecs))
            } else {
                quad_root.find_leaf_id(&mouse_pos)
            };
            let angle = macroquad::rand::gen_range(0.0, std::f32::consts::PI * 2.0);
            sim.spawn_player(mx, my, initial_shard_id, angle);
        }

        if auto_move {
            sim.sim_time += dt;
            sys_auto_move(&mut sim.ecs, dt, screen_w, screen_h, selected_player);
        }

        let mut current_voronoi_data = HashMap::new();

        // ==============================
        //      MODE VORONOI (PUREMENT ECS)
        // ==============================
        if mode == AppMode::Voronoi {
            let metrics = sys_build_metrics(&sim.ecs);

            sys_dynamic_sharding(&mut sim, &metrics);
            sys_merge_shards(&mut sim, &metrics);
            sys_relax_shards(&mut sim, &metrics, 0.05);

            // Rendu Voronoï et logique réseau
            current_voronoi_data = calculate_voronoi_data(&sim.triangulation, screen_w, screen_h, ghost_margin, hysteresis_margin);
            let shards_vec = get_shards_vec(&sim.ecs); // Utilisé uniquement pour l'affichage

            draw_voronoi_polygons(&shards_vec, &current_voronoi_data);
            sys_evaluate_handoffs(&mut sim, &current_voronoi_data, hysteresis_margin);

            for shard in shards_vec {
                let color = get_shard_color(shard.id);
                draw_circle(shard.pos.x, shard.pos.y, 8.0, color);
                draw_circle_lines(shard.pos.x, shard.pos.y, 10.0, 2.0, WHITE);
            }
        }

        // ==============================
        //      MODE QUADTREE (LEGACY)
        // ==============================
        if mode == AppMode::Quadtree {
            // Pont temporaire pour la compatibilité
            let mut players_vec = Vec::new();
            for (e, (p, p_info)) in sim.ecs.query::<(&Position, &PlayerInfo)>().iter() {
                players_vec.push(Player {
                    id: e.to_bits().get() as u32,
                    pos: Point2D { x: p.x, y: p.y },
                    current_shard_id: p_info.current_shard_id,
                    angle: 0.0
                });
            }

            // --- LA CORRECTION EST ICI ---
            // On enferme la collecte dans un bloc pour que l'emprunt meure tout de suite après
            let old_leaves_count = {
                let mut old_leaves = Vec::new();
                quad_root.collect_leaves(&mut old_leaves);
                old_leaves.len() as i32
            }; // Les références immuables (&QuadNode) sont détruites à cette accolade.

            let id_before = sim.next_shard_id;

            // L'emprunt mutable est maintenant parfaitement autorisé
            quad_root.update(&players_vec, &mut sim.next_shard_id, sim.current_tick);

            let mut new_leaves = Vec::new();
            quad_root.collect_leaves(&mut new_leaves);
            let new_leaves_count = new_leaves.len() as i32;

            let splits = (sim.next_shard_id - id_before) / 4;
            sim.stats_splits += splits;
            sim.stats_merges += (splits as i32 - ((new_leaves_count - old_leaves_count) / 3)).max(0) as u32;

            for (_, (pos, player)) in sim.ecs.query_mut::<(&Position, &mut PlayerInfo)>() {
                let old_shard = player.current_shard_id;
                player.current_shard_id = quad_root.find_leaf_id(&Point2D { x: pos.x, y: pos.y });
                if player.current_shard_id != old_shard { sim.stats_handoffs += 1; }
            }

            for leaf in new_leaves {
                let mut color = get_shard_color(leaf.id); color.a = 0.3;
                draw_rectangle(leaf.rect.x, leaf.rect.y, leaf.rect.w, leaf.rect.h, color);
                draw_rectangle_lines(leaf.rect.x, leaf.rect.y, leaf.rect.w, leaf.rect.h, 2.0, WHITE);
            }
        }

        // ==============================
        //  RENDU DEBUG RÉSEAU (AABB)
        // ==============================
        if show_network && mode == AppMode::Voronoi {
            for x in (0..screen_w as u32).step_by(chunk_size as usize) { draw_line(x as f32, 0.0, x as f32, screen_h, 1.0, Color::new(1.0, 1.0, 1.0, 0.10)); }
            for y in (0..screen_h as u32).step_by(chunk_size as usize) { draw_line(0.0, y as f32, screen_w, y as f32, 1.0, Color::new(1.0, 1.0, 1.0, 0.10)); }

            for shard in get_shards_vec(&sim.ecs) {
                if let Some(cell) = current_voronoi_data.get(&shard.id) {
                    let mut color = get_shard_color(shard.id); color.a = 0.15;
                    draw_rectangle(cell.aabb.x, cell.aabb.y, cell.aabb.w, cell.aabb.h, color); color.a = 0.8;
                    draw_rectangle_lines(cell.aabb.x, cell.aabb.y, cell.aabb.w, cell.aabb.h, 2.0, color);

                    for (_, (pos, player)) in sim.ecs.query::<(&Position, &PlayerInfo)>().iter() {
                        if player.current_shard_id != shard.id && cell.aabb.contains(Vec2::new(pos.x, pos.y)) {
                            draw_line(pos.x, pos.y, shard.pos.x, shard.pos.y, 1.0, color);
                            draw_circle_lines(pos.x, pos.y, 22.0, 2.0, color);
                        }
                    }
                }
            }
        }

        // --- RENDU DES JOUEURS ---
        for (entity, (pos, player)) in sim.ecs.query::<(&Position, &PlayerInfo)>().iter() {
            let owner_color = get_shard_color(player.current_shard_id);
            let fill_color = if Some(entity) == selected_player { YELLOW } else { WHITE };
            draw_circle(pos.x, pos.y, 15.0, fill_color);
            draw_circle_lines(pos.x, pos.y, 15.0, 4.0, owner_color);
        }

        // --- UI ---
        let mode_str = if mode == AppMode::Voronoi { "VORONOI (Data-Oriented ECS)" } else { "QUADTREE" };
        let active_servers = if mode == AppMode::Voronoi { get_shards_vec(&sim.ecs).len() } else {
            let mut l = Vec::new(); quad_root.collect_leaves(&mut l); l.len()
        };

        let player_count = sim.ecs.query::<&PlayerInfo>().into_iter().count();

        draw_text(&format!("Mode: {} | FPS: {} | Joueurs: {} | Serveurs: {}", mode_str, get_fps(), player_count, active_servers), 10.0, 20.0, 20.0, WHITE);
        draw_text("Espace: Auto-Move | N: Afficher AABB Réseau | Echap: Menu", 10.0, 45.0, 20.0, YELLOW);

        draw_rectangle(10.0, 70.0, 350.0, 150.0, Color::new(0.0, 0.0, 0.0, 0.8));
        draw_rectangle_lines(10.0, 70.0, 350.0, 150.0, 2.0, GRAY);
        draw_text(&format!("Chrono : {:02}:{:02}", (sim.sim_time / 60.0).floor() as u32, (sim.sim_time % 60.0) as u32), 20.0, 105.0, 25.0, WHITE);
        draw_text(&format!("Transferts : {}", sim.stats_handoffs), 20.0, 140.0, 20.0, ORANGE);
        draw_text(&format!("Éclatements : {}", sim.stats_splits), 20.0, 170.0, 20.0, RED);
        draw_text(&format!("Fusions : {}", sim.stats_merges), 20.0, 200.0, 20.0, GREEN);

        next_frame().await
    }
}