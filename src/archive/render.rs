// src/render.rs
use macroquad::prelude::*;
use std::collections::HashMap;
use crate::voronoi::{Shard, VoronoiCellData};

const PALETTE: [Color; 10] = [RED, GREEN, BLUE, MAGENTA, ORANGE, YELLOW, DARKGREEN, PURPLE, PINK, SKYBLUE];

pub fn get_shard_color(id: u32) -> Color {
    PALETTE[(id as usize) % PALETTE.len()]
}

pub fn draw_voronoi_polygons(shards: &[Shard], cells: &HashMap<u32, VoronoiCellData>) {
    for shard in shards {
        if let Some(cell) = cells.get(&shard.id) {
            let mut color = get_shard_color(shard.id);
            color.a = 0.3;

            let center = vec2(shard.pos.x, shard.pos.y);
            let poly = &cell.polygon;

            if poly.len() >= 3 {
                for i in 0..poly.len() {
                    let v1 = poly[i];
                    let v2 = poly[(i + 1) % poly.len()];

                    draw_triangle(
                        center,
                        vec2(v1.x, v1.y),
                        vec2(v2.x, v2.y),
                        color
                    );
                }
            }
        }
    }
}