// src/voronoi.rs
use macroquad::prelude::Rect;
use rustc_hash::{FxHashMap, FxHashSet};
use spade::{DelaunayTriangulation, HasPosition, Point2, Triangulation};

// --- Structures de base (inchangées) ---
#[derive(Debug, Clone, Copy, Default)]
pub struct Point2D { pub x: f32, pub y: f32 }
impl Point2D {
    pub fn distance_sq(&self, other: &Point2D) -> f32 {
        let dx = self.x - other.x; let dy = self.y - other.y; dx * dx + dy * dy
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ServerVertex { pub id: u32, pub point: Point2<f64> }
impl HasPosition for ServerVertex {
    type Scalar = f64; fn position(&self) -> Point2<f64> { self.point }
}

#[derive(Debug, Clone)]
pub struct Shard { pub id: u32, pub pos: Point2D, pub spawn_tick: u64 }

#[derive(Debug, Clone)]
pub struct VoronoiCellData {
    pub aabb: Rect, pub polygon: Vec<Point2D>, pub neighbors: Vec<u32>, pub safe_radius_sq: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ShardMetrics {
    pub count: u32,
    pub centroid: Point2D,
    pub min_bound: Point2D,
    pub max_bound: Point2D,
}

impl Default for ShardMetrics {
    fn default() -> Self {
        Self {
            count: 0,
            centroid: Point2D::default(),
            min_bound: Point2D { x: f32::MAX, y: f32::MAX },
            max_bound: Point2D { x: f32::MIN, y: f32::MIN },
        }
    }
}

// ==========================================
// LE DROP-IN REPLACEMENT POUR QUADTREE
// ==========================================
pub struct VoronoiMap {
    pub bounds: Rect,
    pub triangulation: DelaunayTriangulation<ServerVertex>,
    pub shards: FxHashMap<u32, Shard>,
    pub cells: FxHashMap<u32, VoronoiCellData>,
    pub players: FxHashMap<u32, Point2D>, // Equivalent de ton AHashMap<ClientId, Vec2<f32>>
}

impl VoronoiMap {
    pub fn new(bounds: Rect) -> Self {
        let mut triangulation = DelaunayTriangulation::new();
        let margin = 5000.0;
        let ghosts = vec![
            Point2::new(-margin, -margin), Point2::new(bounds.w as f64 / 2.0, -margin), Point2::new(bounds.w as f64 + margin, -margin),
            Point2::new(bounds.w as f64 + margin, bounds.h as f64 / 2.0), Point2::new(bounds.w as f64 + margin, bounds.h as f64 + margin),
            Point2::new(bounds.w as f64 / 2.0, bounds.h as f64 + margin), Point2::new(-margin, bounds.h as f64 + margin), Point2::new(-margin, bounds.h as f64 / 2.0),
        ];
        for (i, p) in ghosts.into_iter().enumerate() { triangulation.insert(ServerVertex { id: 1_000_000 + i as u32, point: p }).unwrap(); }

        Self { bounds, triangulation, shards: FxHashMap::default(), cells: FxHashMap::default(), players: FxHashMap::default() }
    }

    // --- LES METHODES DE TON QUADTREE ACTUEL ---

    pub fn insert_player(&mut self, client_id: u32, pos: Point2D) -> Option<u32> {
        self.players.insert(client_id, pos);
        self.shard_id_for(pos)
    }

    pub fn remove_player(&mut self, client_id: u32, _shard_id: u32) -> Option<Point2D> {
        self.players.remove(&client_id)
    }

    pub fn hard_update_player_position(&mut self, client_id: u32, _shard_id: u32, new_pos: Point2D) -> Option<Point2D> {
        self.players.insert(client_id, new_pos) // Retourne l'ancienne pos
    }

    /// Avec Voronoï, l'ID d'un point est simplement le centre de gravité (shard) le plus proche !
    pub fn shard_id_for(&self, pos: Point2D) -> Option<u32> {
        let mut min_dist = f32::MAX;
        let mut nearest_id = None;
        for shard in self.shards.values() {
            let dist = pos.distance_sq(&shard.pos);
            if dist < min_dist { min_dist = dist; nearest_id = Some(shard.id); }
        }
        nearest_id
    }

    /// Récupère tous les shards qui intersectent la zone de Ghost/Interest du joueur
    pub fn shards_near(&self, pos: Point2D, margin: f32) -> Vec<u32> {
        let mut result = Vec::new();
        let query_rect = Rect::new(pos.x - margin, pos.y - margin, margin * 2.0, margin * 2.0);

        for (id, cell) in &self.cells {
            if cell.aabb.overlaps(&query_rect) { result.push(*id); }
        }
        result
    }

    // --- GESTION DES SERVEURS ---
    pub fn spawn_shard(&mut self, id: u32, pos: Point2D, tick: u64) {
        self.shards.insert(id, Shard { id, pos, spawn_tick: tick });
        self.triangulation.insert(ServerVertex { id, point: Point2::new(pos.x as f64, pos.y as f64) }).unwrap();
    }

    pub fn despawn_shard(&mut self, id: u32) {
        if let Some(shard) = self.shards.remove(&id) {
            let pt = Point2::new(shard.pos.x as f64, shard.pos.y as f64);
            self.triangulation.locate_and_remove(pt);
        }
    }

    pub fn move_shard_vertex(&mut self, id: u32, new_x: f32, new_y: f32) {
        if let Some(shard) = self.shards.get_mut(&id) {
            let old_pt = Point2::new(shard.pos.x as f64, shard.pos.y as f64);
            if self.triangulation.locate_and_remove(old_pt).is_some() {
                self.triangulation.insert(ServerVertex { id, point: Point2::new(new_x as f64, new_y as f64) }).unwrap();
                shard.pos.x = new_x; shard.pos.y = new_y;
            }
        }
    }

    pub fn build_metrics(&self) -> FxHashMap<u32, ShardMetrics> {
        let mut metrics: FxHashMap<u32, ShardMetrics> = FxHashMap::default();

        for &pos in self.players.values() {
            if let Some(shard_id) = self.shard_id_for(pos) {
                let m = metrics.entry(shard_id).or_default();
                m.count += 1;
                m.centroid.x += pos.x;
                m.centroid.y += pos.y;

                m.min_bound.x = m.min_bound.x.min(pos.x);
                m.min_bound.y = m.min_bound.y.min(pos.y);
                m.max_bound.x = m.max_bound.x.max(pos.x);
                m.max_bound.y = m.max_bound.y.max(pos.y);
            }
        }
        metrics
    }

    /// Applique l'algorithme de Lloyd : déplace les serveurs vers le centre de masse de leurs joueurs
    pub fn relax_shards(&mut self, metrics: &FxHashMap<u32, ShardMetrics>, lerp_factor: f32) {
        let mut updates = Vec::new();

        // 1. On calcule les nouvelles positions (On lit self.shards)
        for shard in self.shards.values() {
            if let Some(m) = metrics.get(&shard.id) {
                if m.count > 0 {
                    let cx = m.centroid.x / (m.count as f32);
                    let cy = m.centroid.y / (m.count as f32);
                    let dx = cx - shard.pos.x;
                    let dy = cy - shard.pos.y;

                    if dx * dx + dy * dy > 25.0 {
                        updates.push((shard.id, shard.pos.x + dx * lerp_factor, shard.pos.y + dy * lerp_factor));
                    }
                }
            }
        }

        // 2. On applique les mouvements (On mute self.triangulation via notre méthode)
        for (id, nx, ny) in updates {
            self.move_shard_vertex(id, nx, ny);
        }
    }

    /// Génère le diagramme de Voronoï clippé et met à jour `self.cells` directement.
    pub fn update_voronoi_data(&mut self, ghost_margin: f32, hysteresis_margin: f32) {
        let mut new_cells = FxHashMap::default();

        // On récupère les limites extrêmes de la map
        let min_bound = Point2D { x: self.bounds.x, y: self.bounds.y };
        let max_bound = Point2D { x: self.bounds.x + self.bounds.w, y: self.bounds.y + self.bounds.h };

        for vertex in self.triangulation.vertices() {
            let shard_id = vertex.data().id;
            if shard_id >= 1_000_000 { continue; } // Ignorer les ghost points

            let face = vertex.as_voronoi_face();
            let mut raw_polygon = Vec::with_capacity(8);

            // Construction du polygone brut
            for edge in face.adjacent_edges() {
                if let spade::handles::VoronoiVertex::Inner(inner) = edge.from() {
                    let p = inner.circumcenter();
                    raw_polygon.push(Point2D { x: p.x as f32, y: p.y as f32 });
                }
            }

            // Clipping du polygone contre les bords de la map
            let clipped_polygon = clip_polygon_aabb(&raw_polygon, &min_bound, &max_bound);
            if clipped_polygon.is_empty() { continue; }

            // Calcul de l'AABB du polygone clippé
            let mut min_x = f32::MAX; let mut min_y = f32::MAX;
            let mut max_x = f32::MIN; let mut max_y = f32::MIN;

            for p in &clipped_polygon {
                min_x = min_x.min(p.x); min_y = min_y.min(p.y);
                max_x = max_x.max(p.x); max_y = max_y.max(p.y);
            }

            // Application du Ghost Margin tout en restant dans la carte
            let final_min_x = (min_x - ghost_margin).max(self.bounds.x);
            let final_min_y = (min_y - ghost_margin).max(self.bounds.y);
            let final_max_x = (max_x + ghost_margin).min(self.bounds.x + self.bounds.w);
            let final_max_y = (max_y + ghost_margin).min(self.bounds.y + self.bounds.h);

            let rect_w = (final_max_x - final_min_x).max(1.0);
            let rect_h = (final_max_y - final_min_y).max(1.0);

            // Calcul du rayon de sécurité pour la Fast-Path (Handoffs O(1))
            let mut neighbors = Vec::new();
            let mut min_neighbor_dist_sq = f32::MAX;

            for edge in vertex.out_edges() {
                let n_id = edge.to().data().id;
                if n_id < 1_000_000 {
                    neighbors.push(n_id);

                    let n_pos = edge.to().position();
                    let dx = n_pos.x as f32 - vertex.position().x as f32;
                    let dy = n_pos.y as f32 - vertex.position().y as f32;
                    let dist_sq = dx * dx + dy * dy;
                    if dist_sq < min_neighbor_dist_sq {
                        min_neighbor_dist_sq = dist_sq;
                    }
                }
            }

            let min_dist = min_neighbor_dist_sq.sqrt();
            let actual_safe_radius = (min_dist / 2.0) - hysteresis_margin;
            let safe_radius_sq = if actual_safe_radius > 0.0 { actual_safe_radius * actual_safe_radius } else { 0.0 };

            new_cells.insert(shard_id, VoronoiCellData {
                aabb: Rect::new(final_min_x, final_min_y, rect_w, rect_h),
                polygon: clipped_polygon,
                neighbors,
                safe_radius_sq,
            });
        }

        // On remplace les anciennes cellules par les nouvelles
        self.cells = new_cells;
    }
}

pub fn clip_polygon_aabb(poly: &[Point2D], min_b: &Point2D, max_b: &Point2D) -> Vec<Point2D> {
    let mut input = poly.to_vec();
    let mut output = Vec::with_capacity(8);

    // Axe X - Gauche
    clip_edge(&mut input, &mut output, |p| p.x >= min_b.x, |p1, p2| {
        let t = (min_b.x - p1.x) / (p2.x - p1.x);
        Point2D { x: min_b.x, y: p1.y + t * (p2.y - p1.y) }
    });
    std::mem::swap(&mut input, &mut output); output.clear();

    // Axe X - Droite
    clip_edge(&mut input, &mut output, |p| p.x <= max_b.x, |p1, p2| {
        let t = (max_b.x - p1.x) / (p2.x - p1.x);
        Point2D { x: max_b.x, y: p1.y + t * (p2.y - p1.y) }
    });
    std::mem::swap(&mut input, &mut output); output.clear();

    // Axe Y - Haut
    clip_edge(&mut input, &mut output, |p| p.y >= min_b.y, |p1, p2| {
        let t = (min_b.y - p1.y) / (p2.y - p1.y);
        Point2D { x: p1.x + t * (p2.x - p1.x), y: min_b.y }
    });
    std::mem::swap(&mut input, &mut output); output.clear();

    // Axe Y - Bas
    clip_edge(&mut input, &mut output, |p| p.y <= max_b.y, |p1, p2| {
        let t = (max_b.y - p1.y) / (p2.y - p1.y);
        Point2D { x: p1.x + t * (p2.x - p1.x), y: max_b.y }
    });

    output
}

#[inline(always)]
fn clip_edge<F, I>(input: &mut Vec<Point2D>, output: &mut Vec<Point2D>, inside: F, intersect: I)
where
    F: Fn(&Point2D) -> bool,
    I: Fn(&Point2D, &Point2D) -> Point2D,
{
    if input.is_empty() { return; }
    let mut prev = *input.last().unwrap();
    let mut prev_in = inside(&prev);

    for curr in input.iter() {
        let curr_in = inside(curr);
        if curr_in != prev_in { output.push(intersect(&prev, curr)); } // Franchissement de frontière : on crée un sommet d'intersection
        if curr_in { output.push(*curr); } // Point à l'intérieur : on le garde
        prev = *curr;
        prev_in = curr_in;
    }
}