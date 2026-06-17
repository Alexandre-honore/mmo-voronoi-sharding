// src/engine.rs
use hecs::{Entity, World};
use spade::{DelaunayTriangulation, HasPosition, Point2, Triangulation};

// ==============================
//    TYPES SPADE (VORONOI)
// ==============================
#[derive(Clone, PartialEq, Debug)]
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

// ==============================
//          COMPOSANTS
// ==============================
#[derive(Debug, Clone, Copy)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Velocity {
    pub vx: f32,
    pub vy: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerInfo {
    pub current_shard_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ShardInfo {
    pub id: u32,
    pub spawn_tick: u64,
}

// ==============================
//        ÉTAT DU MONDE
// ==============================
pub struct SimWorld {
    pub ecs: World,
    pub triangulation: DelaunayTriangulation<ServerVertex>,
    pub current_tick: u64,
    pub next_shard_id: u32,
    pub next_player_id: u32,
    pub stats_handoffs: u32,
    pub stats_splits: u32,
    pub stats_merges: u32,
    pub sim_time: f32,
}

impl SimWorld {
    pub fn new(map_w: f32, map_h: f32) -> Self {
        let mut triangulation = DelaunayTriangulation::new();

        // Création des GHOST POINTS GÉANTS une seule fois à l'initialisation.
        let w = map_w as f64;
        let h = map_h as f64;
        let margin = 5000.0;
        let ghosts = vec![
            Point2::new(-margin, -margin), Point2::new(w / 2.0, -margin), Point2::new(w + margin, -margin),
            Point2::new(w + margin, h / 2.0), Point2::new(w + margin, h + margin), Point2::new(w / 2.0, h + margin),
            Point2::new(-margin, h + margin), Point2::new(-margin, h / 2.0),
        ];

        for (i, p) in ghosts.into_iter().enumerate() {
            triangulation.insert(ServerVertex { id: 1_000_000 + i as u32, point: p }).unwrap();
        }

        Self {
            ecs: World::new(),
            triangulation,
            current_tick: 0,
            next_shard_id: 2,
            next_player_id: 101,
            stats_handoffs: 0,
            stats_splits: 0,
            stats_merges: 0,
            sim_time: 0.0,
        }
    }

    pub fn spawn_player(&mut self, x: f32, y: f32, shard_id: u32, angle: f32) {
        self.ecs.spawn((
            Position { x, y },
            Velocity { vx: angle.cos(), vy: angle.sin() },
            PlayerInfo { current_shard_id: shard_id },
        ));
    }

    pub fn spawn_shard(&mut self, x: f32, y: f32, id: u32, tick: u64) {
        self.ecs.spawn((
            Position { x, y },
            ShardInfo { id, spawn_tick: tick },
        ));
        self.triangulation.insert(ServerVertex { id, point: Point2::new(x as f64, y as f64) }).unwrap();
    }

    pub fn despawn_shard(&mut self, entity: Entity, pos: &Position) {
        let pt = Point2::new(pos.x as f64, pos.y as f64);
        self.triangulation.locate_and_remove(pt);
        self.ecs.despawn(entity).unwrap();
    }

    pub fn move_shard(&mut self, entity: Entity, old_pos: &Position, new_x: f32, new_y: f32) {
        let pt = Point2::new(old_pos.x as f64, old_pos.y as f64);
        let id = self.ecs.get::<&ShardInfo>(entity).unwrap().id;

        if self.triangulation.locate_and_remove(pt).is_some() {
            self.triangulation.insert(ServerVertex { id, point: Point2::new(new_x as f64, new_y as f64) }).unwrap();
        }

        let mut pos = self.ecs.get::<&mut Position>(entity).unwrap();
        pos.x = new_x;
        pos.y = new_y;
    }
}

// ==============================
//           SYSTÈMES
// ==============================
pub fn sys_auto_move(world: &mut World, dt: f32, bounds_w: f32, bounds_h: f32, selected_entity: Option<Entity>) {
    let speed = 150.0;
    for (entity, (pos, vel)) in world.query_mut::<(&mut Position, &mut Velocity)>() {
        if Some(entity) == selected_entity { continue; }
        pos.x += vel.vx * speed * dt;
        pos.y += vel.vy * speed * dt;
        if pos.x < 15.0 || pos.x > bounds_w - 15.0 { vel.vx = -vel.vx; pos.x = pos.x.clamp(15.0, bounds_w - 15.0); }
        if pos.y < 15.0 || pos.y > bounds_h - 15.0 { vel.vy = -vel.vy; pos.y = pos.y.clamp(15.0, bounds_h - 15.0); }
    }
}