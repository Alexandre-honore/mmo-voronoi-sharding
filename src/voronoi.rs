// src/voronoi.rs
use std::collections::HashMap;
use macroquad::prelude::Rect;
use spade::{DelaunayTriangulation, HasPosition, Point2, Triangulation};

#[derive(Debug, Clone, Copy)]
pub struct Point2D {
    pub x: f32,
    pub y: f32,
}

impl Point2D {
    #[inline]
    pub fn distance_sq(&self, other: &Point2D) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone)]
pub struct Shard {
    pub id: u32,
    pub pos: Point2D,
    pub spawn_tick: u64,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub id: u32,
    pub pos: Point2D,
    pub current_shard_id: u32,
}

#[derive(Debug, Clone)]
pub struct VoronoiCellData {
    pub aabb: Rect,
    pub polygon: Vec<Point2D>,
}

#[derive(Clone, PartialEq)]
pub struct ServerVertex {
    pub id: u32,
    pub point: Point2<f64>,
}

impl HasPosition for ServerVertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        self.point
    }
}

pub fn calculate_voronoi_data(
    shards: &[Shard],
    map_width: f32,
    map_height: f32,
    ghost_margin: f32,
) -> HashMap<u32, VoronoiCellData> {
    let mut cells = HashMap::new();
    if shards.is_empty() { return cells; }

    let mut triangulation: DelaunayTriangulation<ServerVertex> = DelaunayTriangulation::new();

    let w = map_width as f64;
    let h = map_height as f64;
    
    for shard in shards {
        let id = shard.id;
        let x = shard.pos.x as f64;
        let y = shard.pos.y as f64;
        
        triangulation.insert(ServerVertex { id, point: Point2::new(x, y) }).unwrap();
        
        let dummy = id + 1_000_000;

        // Reflets sur les 4 murs (Garantit que les bords sont parfaitement coupés droits)
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(-x, y) }).unwrap(); // Mur Gauche
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(2.0 * w - x, y) }).unwrap(); // Mur Droit
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(x, -y) }).unwrap(); // Plafond
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(x, 2.0 * h - y) }).unwrap(); // Sol

        // Reflets dans les 4 coins (Garantit que les angles morts sont fermés)
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(-x, -y) }).unwrap();
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(2.0 * w - x, -y) }).unwrap();
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(-x, 2.0 * h - y) }).unwrap();
        triangulation.insert(ServerVertex { id: dummy, point: Point2::new(2.0 * w - x, 2.0 * h - y) }).unwrap();
    }

    for vertex in triangulation.vertices() {
        let shard_id = vertex.data().id;

        // On ignore totalement la géométrie des miroirs
        if shard_id >= 1_000_000 { continue; }

        let face = vertex.as_voronoi_face();
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        let mut polygon = Vec::new();
        
        for edge in face.adjacent_edges() {
            if let spade::handles::VoronoiVertex::Inner(inner) = edge.from() {
                let p = inner.circumcenter();
                polygon.push(Point2D { x: p.x as f32, y: p.y as f32 });

                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }

        let final_min_x = (min_x as f32 - ghost_margin).max(0.0);
        let final_min_y = (min_y as f32 - ghost_margin).max(0.0);
        let final_max_x = (max_x as f32 + ghost_margin).min(map_width);
        let final_max_y = (max_y as f32 + ghost_margin).min(map_height);

        let rect_w = (final_max_x - final_min_x).max(1.0);
        let rect_h = (final_max_y - final_min_y).max(1.0);

        cells.insert(shard_id, VoronoiCellData {
            aabb: Rect::new(final_min_x, final_min_y, rect_w, rect_h),
            polygon,
        });
    }

    cells
}

pub fn find_nearest_shard_id(pos: &Point2D, shards: &[Shard]) -> u32 {
    let mut min_dist = f32::MAX;
    let mut nearest_id = shards[0].id;

    for shard in shards {
        let dist = pos.distance_sq(&shard.pos); // Géométrie pure, plus de weight
        if dist < min_dist {
            min_dist = dist;
            nearest_id = shard.id;
        }
    }
    nearest_id
}

pub fn evaluate_handoff(player: &Player, shards: &[Shard], hysteresis_margin: f32) -> u32 {
    let current_shard = shards.iter().find(|s| s.id == player.current_shard_id);
    let current_shard = match current_shard {
        Some(s) => s,
        None => return find_nearest_shard_id(&player.pos, shards),
    };

    let current_dist_sq = player.pos.distance_sq(&current_shard.pos);
    let best_id = find_nearest_shard_id(&player.pos, shards);

    if best_id != player.current_shard_id {
        let best_shard = shards.iter().find(|s| s.id == best_id).unwrap();
        let best_dist_sq = player.pos.distance_sq(&best_shard.pos);
        let margin_sq = hysteresis_margin * hysteresis_margin;

        if best_dist_sq < (current_dist_sq - margin_sq) {
            return best_id;
        }
    }
    player.current_shard_id
}

pub fn update_dynamic_sharding(shards: &mut Vec<Shard>, players: &mut [Player], next_shard_id: &mut u32, current_tick: u64) -> bool {
    let mut shard_populations: HashMap<u32, Vec<Point2D>> = HashMap::new();
    for player in players.iter() {
        shard_populations.entry(player.current_shard_id).or_default().push(player.pos);
    }

    let mut split_occurred = false;
    let mut shards_to_remove = Vec::new();
    let mut new_shards = Vec::new();

    for (shard_id, player_positions) in shard_populations {
        let shard = shards.iter().find(|s| s.id == shard_id).unwrap();

        if player_positions.len() >= 5 && current_tick > shard.spawn_tick + 60 {
            split_occurred = true;
            shards_to_remove.push(shard_id);

            let mut max_dist_sq = -1.0;
            let mut p1 = player_positions[0];
            let mut p2 = player_positions[1];

            for i in 0..player_positions.len() {
                for j in (i + 1)..player_positions.len() {
                    let dist = player_positions[i].distance_sq(&player_positions[j]);
                    if dist > max_dist_sq {
                        max_dist_sq = dist;
                        p1 = player_positions[i];
                        p2 = player_positions[j];
                    }
                }
            }

            new_shards.push(Shard { id: *next_shard_id, pos: p1, spawn_tick: current_tick });
            *next_shard_id += 1;
            new_shards.push(Shard { id: *next_shard_id, pos: p2, spawn_tick: current_tick });
            *next_shard_id += 1;
        }
    }

    if split_occurred {
        shards.retain(|s| !shards_to_remove.contains(&s.id));
        shards.extend(new_shards);

        for player in players.iter_mut() {
            player.current_shard_id = find_nearest_shard_id(&player.pos, shards);
        }
    }
    split_occurred
}

pub fn relax_shards(shards: &mut [Shard], players: &[Player], lerp_factor: f32) -> bool {
    let mut moved = false;
    let mut centroids: HashMap<u32, Point2D> = HashMap::new();
    let mut counts: HashMap<u32, u32> = HashMap::new();

    for p in players {
        let c = centroids.entry(p.current_shard_id).or_insert(Point2D { x: 0.0, y: 0.0 });
        c.x += p.pos.x;
        c.y += p.pos.y;
        *counts.entry(p.current_shard_id).or_insert(0) += 1;
    }

    for shard in shards.iter_mut() {
        if let Some(count) = counts.get(&shard.id) {
            let cx = centroids[&shard.id].x / (*count as f32);
            let cy = centroids[&shard.id].y / (*count as f32);

            let dx = cx - shard.pos.x;
            let dy = cy - shard.pos.y;

            if dx * dx + dy * dy > 1.0 {
                shard.pos.x += dx * lerp_factor;
                shard.pos.y += dy * lerp_factor;
                moved = true;
            }
        }
    }
    moved
}

pub fn merge_underpopulated_shards(shards: &mut Vec<Shard>, players: &[Player], next_shard_id: &mut u32, current_tick: u64) -> bool {
    if shards.len() <= 1 { return false; }

    let mut counts: HashMap<u32, u32> = HashMap::new();
    for shard in shards.iter() { counts.insert(shard.id, 0); }
    for player in players {
        if let Some(c) = counts.get_mut(&player.current_shard_id) { *c += 1; }
    }

    let mut best_pair: Option<(usize, usize)> = None;
    let mut min_dist_sq = f32::MAX;
    let max_merge_dist_sq = 400.0 * 400.0;

    for i in 0..shards.len() {
        for j in (i + 1)..shards.len() {
            let shard_i = &shards[i];
            let shard_j = &shards[j];

            if current_tick < shard_i.spawn_tick + 60 || current_tick < shard_j.spawn_tick + 60 {
                continue;
            }

            let count_i = counts[&shard_i.id];
            let count_j = counts[&shard_j.id];

            if count_i + count_j <= 2 {
                let dist_sq = shard_i.pos.distance_sq(&shard_j.pos);
                if dist_sq < min_dist_sq && dist_sq < max_merge_dist_sq {
                    min_dist_sq = dist_sq;
                    best_pair = Some((i, j));
                }
            }
        }
    }

    if let Some((i, j)) = best_pair {
        let id_i = shards[i].id;
        let id_j = shards[j].id;

        let new_pos = Point2D {
            x: (shards[i].pos.x + shards[j].pos.x) / 2.0,
            y: (shards[i].pos.y + shards[j].pos.y) / 2.0,
        };

        shards.retain(|s| s.id != id_i && s.id != id_j);
        shards.push(Shard { id: *next_shard_id, pos: new_pos, spawn_tick: current_tick });
        *next_shard_id += 1;

        return true;
    }
    false
}