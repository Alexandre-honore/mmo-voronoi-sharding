use macroquad::prelude::*;
use crate::shared::{Player, PlayerKey, Shard, ShardKey, Point2D, AABB};
use std::collections::HashMap;

// Ta palette simplifiée
const PALETTE: [Color; 10] = [
    RED, GREEN, BLUE, MAGENTA, ORANGE,
    YELLOW, DARKGREEN, PURPLE, PINK, SKYBLUE
];

// La map ne stocke plus qu'une seule couleur par Shard
pub fn build_color_map<'a>(shards: impl Iterator<Item = (ShardKey, &'a Shard)>) -> HashMap<ShardKey, Color> {
    let mut map = HashMap::new();
    for (i, (key, _)) in shards.enumerate() {
        let color = PALETTE[i % PALETTE.len()];
        map.insert(key, color);
    }
    map
}

fn draw_convex_polygon(poly: &[Point2D], color: Color) {
    if poly.len() < 3 { return; }
    let p0 = Vec2::new(poly[0].x, poly[0].y);
    for i in 1..(poly.len() - 1) {
        let p1 = Vec2::new(poly[i].x, poly[i].y);
        let p2 = Vec2::new(poly[i+1].x, poly[i+1].y);
        draw_triangle(p0, p1, p2, color);
    }
}

pub fn draw_voronoi(polygons: &[(ShardKey, Vec<Point2D>)], color_map: &HashMap<ShardKey, Color>) {
    for (key, poly) in polygons {
        if let Some(&base_color) = color_map.get(key) {
            // Création de la couleur de fond en écrasant uniquement l'alpha (0.3)
            let bg_color = Color { a: 0.3, ..base_color };

            draw_convex_polygon(poly, bg_color);

            for i in 0..poly.len() {
                let p1 = poly[i];
                let p2 = poly[(i + 1) % poly.len()];
                draw_line(p1.x, p1.y, p2.x, p2.y, 1.0, bg_color);
            }
        }
    }
}

pub fn draw_shards<'a>(shards: impl Iterator<Item = (ShardKey, &'a Shard)>, color_map: &HashMap<ShardKey, Color>) {
    for (key, shard) in shards {
        if let Some(&color) = color_map.get(&key) {
            // Shard : Point coloré, outline blanc
            draw_circle(shard.pos.x, shard.pos.y, 8.0, color);
            draw_circle_lines(shard.pos.x, shard.pos.y, 8.0, 2.0, WHITE);
        }
    }
}

pub fn draw_players<'a>(players: impl Iterator<Item = (PlayerKey, &'a Player)>, color_map: &HashMap<ShardKey, Color>) {
    for (_key, player) in players {
        if let Some(&color) = color_map.get(&player.current_shard) {
            // Player : Point blanc, outline coloré
            draw_circle(player.pos.x, player.pos.y, 6.0, WHITE);
            draw_circle_lines(player.pos.x, player.pos.y, 6.0, 2.0, color);
        }
    }
}

pub fn draw_ghost_aabbs<'a>(cells: impl Iterator<Item = (ShardKey, &'a crate::spatial::ShardCellData)>, color_map: &HashMap<ShardKey, Color>) {
    for (key, cell) in cells {
        if let Some(&color) = color_map.get(&key) {
            let aabb = &cell.ghost_aabb;
            let w = aabb.max_x - aabb.min_x;
            let h = aabb.max_y - aabb.min_y;

            // On dessine le contour de l'AABB avec la couleur du shard, en très fin (1.0)
            draw_rectangle_lines(aabb.min_x, aabb.min_y, w, h, 1.0, color);
        }
    }
}