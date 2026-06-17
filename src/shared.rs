use slotmap::{new_key_type};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AABB {
    pub min_x: f32, pub min_y: f32,
    pub max_x: f32, pub max_y: f32,
}

impl AABB {
    pub fn contains(&self, p: &Point2D) -> bool {
        p.x >= self.min_x && p.x <= self.max_x && p.y >= self.min_y && p.y <= self.max_y
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Point2D {
    pub x: f32,
    pub y: f32,
}

impl Point2D {
    #[inline]
    pub fn distance_sq(&self, other: &Point2D) -> f32 {
        // Optimisation : On utilise la distance au carré pour éviter
        // de calculer une racine carrée coûteuse (théorème de Pythagore).
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

new_key_type! {
    pub struct PlayerKey;
    pub struct ShardKey;
}

pub struct Player {
    pub pos: Point2D,
    pub current_shard: ShardKey,
    pub ghost_shards: Vec<ShardKey>,
}

pub struct Shard {
    pub pos: Point2D,
    pub spawn_tick: u64,
}