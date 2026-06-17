use crate::engine::{PlayerInfo, Position, ServerVertex, ShardInfo, SimWorld};
use hecs::World;
use macroquad::prelude::Rect;
use spade::Triangulation;
use std::collections::HashMap;

// ============================================================================
// NOTE OPTIMISATION (Production) :
// Pour 100 000 joueurs, remplacez `std::collections::HashMap` par `FxHashMap`
// de la crate `rustc-hash`. Le SipHash par défaut de Rust est trop lent pour
// un usage interne temps-réel non exposé aux attaques DDoS.
// ============================================================================

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
    pub angle: f32,
}

#[derive(Debug, Clone)]
pub struct VoronoiCellData {
    pub aabb: Rect,              // Bounding Box exacte du polygone visible
    pub polygon: Vec<Point2D>,   // Polygone découpé (clippé) aux bords de l'écran
    pub neighbors: Vec<u32>,     // ID des shards adjacents
    pub safe_radius_sq: f32,     // Le rayon à partir duquel on évalue les handoffs
}

// ----------------------------------------------------------------------------
// ZÉRO-ALLOCATION METRICS
// ----------------------------------------------------------------------------
// Au lieu d'allouer dynamiquement un Vec<Point2D> pour stocker la position de
// tous les joueurs (ce qui détruirait la RAM/CPU à 100k joueurs), on maintient
// des bornes mathématiques (AABB) calculées en O(1) à la volée.
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

// Extrait un Vec<Shard> temporaire depuis l'ECS (utile pour le rendu)
pub fn get_shards_vec(world: &World) -> Vec<Shard> {
    world.query::<(&Position, &ShardInfo)>().iter().map(|(_, (p, s))| Shard {
        id: s.id, pos: Point2D { x: p.x, y: p.y }, spawn_tick: s.spawn_tick
    }).collect()
}

// ============================================================================
// 1. OUTILS GÉOMÉTRIQUES : ALGORITHME DE SUTHERLAND-HODGMAN
// ============================================================================
// Principe : Algorithme de découpage (clipping) de polygones en complexité $O(N)$.
// Il prend un polygone débordant largement de la carte (à cause des Ghost Points)
// et le tranche comme un massicot contre les 4 plans (Gauche, Droite, Haut, Bas)
// de notre carte (AABB). L'utilisation d'un système de double-buffer (input/output)
// empêche les allocations multiples.
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

// ============================================================================
// 2. GÉNÉRATION DES DONNÉES VORONOÏ ET DUALITÉ DE DELAUNAY
// ============================================================================
pub fn calculate_voronoi_data(
    triangulation: &spade::DelaunayTriangulation<ServerVertex>,
    map_width: f32, map_height: f32, ghost_margin: f32, hysteresis_margin: f32
) -> HashMap<u32, VoronoiCellData> {
    let mut cells = HashMap::with_capacity(triangulation.num_vertices());

    let min_bound = Point2D { x: 0.0, y: 0.0 };
    let max_bound = Point2D { x: map_width, y: map_height };

    for vertex in triangulation.vertices() {
        let shard_id = vertex.data().id;
        if shard_id >= 1_000_000 { continue; } // On ignore la topologie des Ghost Points

        let face = vertex.as_voronoi_face();
        let mut raw_polygon = Vec::with_capacity(8);

        // PRINCIPE DE DUALITÉ MATHEMATIQUE :
        // Le graphe de Voronoï est le diagramme dual de la Triangulation de Delaunay.
        // Les sommets d'un polygone de Voronoï sont exactement les centres des cercles
        // circonscrits (circentres) des triangles de Delaunay adjacents.
        for edge in face.adjacent_edges() {
            if let spade::handles::VoronoiVertex::Inner(inner) = edge.from() {
                let p = inner.circumcenter();
                raw_polygon.push(Point2D { x: p.x as f32, y: p.y as f32 });
            }
        }

        // On clip le polygone avec notre massicot mathématique Sutherland-Hodgman
        let clipped_polygon = clip_polygon_aabb(&raw_polygon, &min_bound, &max_bound);
        if clipped_polygon.is_empty() { continue; }

        // Extraction de l'AABB parfaite basée uniquement sur le polygone clippé
        let mut min_x = f32::MAX; let mut min_y = f32::MAX;
        let mut max_x = f32::MIN; let mut max_y = f32::MIN;

        for p in &clipped_polygon {
            min_x = min_x.min(p.x); min_y = min_y.min(p.y);
            max_x = max_x.max(p.x); max_y = max_y.max(p.y);
        }

        // Application du "Ghost Margin" (Zone de couverture réseau / Interest Management)
        let final_min_x = (min_x - ghost_margin).max(0.0);
        let final_min_y = (min_y - ghost_margin).max(0.0);
        let final_max_x = (max_x + ghost_margin).min(map_width);
        let final_max_y = (max_y + ghost_margin).min(map_height);

        let rect_w = (final_max_x - final_min_x).max(1.0);
        let rect_h = (final_max_y - final_min_y).max(1.0);

        let mut neighbors = Vec::new();
        for edge in vertex.out_edges() {
            let n_id = edge.to().data().id;
            if n_id < 1_000_000 { neighbors.push(n_id); }
        }

        let mut min_neighbor_dist_sq = f32::MAX;
        for edge in vertex.out_edges() {
            let n_id = edge.to().data().id;
            if n_id < 1_000_000 {
                neighbors.push(n_id);
                // Calcule la distance avec le germe voisin
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

        // Le rayon de sécurité est la moitié de cette distance, MOINS ton hystérésis
        let actual_safe_radius = (min_dist / 2.0) - hysteresis_margin;

        // On le remet au carré et on le stocke ! Zéro racine carrée pour les joueurs.
        let safe_radius_sq = if actual_safe_radius > 0.0 {
            actual_safe_radius * actual_safe_radius
        } else {
            0.0
        };

        cells.insert(shard_id, VoronoiCellData {
            aabb: Rect::new(final_min_x, final_min_y, rect_w, rect_h),
            polygon: clipped_polygon,
            neighbors,
            safe_radius_sq,
        });
    }
    cells
}

pub fn find_nearest_shard_id(pos: &Point2D, shards: &[Shard]) -> u32 {
    let mut min_dist = f32::MAX;
    let mut nearest_id = shards.first().map(|s| s.id).unwrap_or(0);
    for shard in shards {
        let dist = pos.distance_sq(&shard.pos);
        if dist < min_dist { min_dist = dist; nearest_id = shard.id; }
    }
    nearest_id
}

// ============================================================================
// 3. TRANSFERTS (HANDOFFS) : CULLING SPATIAL & HYSTÉRÉSIS
// ============================================================================
pub fn evaluate_handoff(
    pos: &Point2D, current_shard_id: u32, shards: &[Shard],
    voronoi_data: &HashMap<u32, VoronoiCellData>, hysteresis_margin: f32
) -> u32 {
    if let Some(cell_data) = voronoi_data.get(&current_shard_id) {
        if let Some(current) = shards.iter().find(|s| s.id == current_shard_id) {

            let current_dist_sq = pos.distance_sq(&current.pos);

            // 1. FAST-PATH VORONOI EXACT : Le Cercle Inscrit
            // On réduit le cercle de la marge d'hystérésis pour être sûr à 100%
            // Si le joueur est là-dedans, c'est réglé en 1 seule opération mathématique.
            if current_dist_sq < cell_data.safe_radius_sq {
                return current.id;
            }

            // 2. SLOW-PATH : Le joueur est dans la "zone grise" près des bords du polygone
            // On doit vérifier les voisins un par un (ton code de base).
            let mut best_dist_sq = current_dist_sq;
            let mut best_id = current.id;

            for &neighbor_id in &cell_data.neighbors {
                if let Some(n_shard) = shards.iter().find(|s| s.id == neighbor_id) {
                    let dist_sq = pos.distance_sq(&n_shard.pos);
                    if dist_sq < best_dist_sq {
                        best_dist_sq = dist_sq;
                        best_id = neighbor_id;
                    }
                }
            }

            if best_id != current.id && best_dist_sq < (current_dist_sq - (hysteresis_margin * hysteresis_margin)) {
                return best_id;
            }
            return current.id;
        }
    }
    find_nearest_shard_id(pos, shards)
}

// ============================================================================
// 4. LOGIQUE ECS (SYSTEMS) ET MÉCANIQUE DES FLUIDES
// ============================================================================
pub fn sys_build_metrics(world: &World) -> HashMap<u32, ShardMetrics> {
    let mut metrics: HashMap<u32, ShardMetrics> = HashMap::with_capacity(64);
    for (_, (pos, player)) in world.query::<(&Position, &PlayerInfo)>().iter() {
        let m = metrics.entry(player.current_shard_id).or_default();
        m.count += 1;
        m.centroid.x += pos.x;
        m.centroid.y += pos.y;

        // Calcul des bornes AABB du nuage de joueurs en temps réel sans allocation.
        m.min_bound.x = m.min_bound.x.min(pos.x);
        m.min_bound.y = m.min_bound.y.min(pos.y);
        m.max_bound.x = m.max_bound.x.max(pos.x);
        m.max_bound.y = m.max_bound.y.max(pos.y);
    }
    metrics
}

// PRINCIPE : ALGORITHME DE LLOYD (Relaxation de Voronoï)
// Repousse les limites de chaque cellule vers le barycentre (centroid) de sa population.
// Si un groupe de joueurs court vers la droite, le centre de masse se déplace à droite,
// et le Shard ("germe" de Voronoï) va glisser (Lerp) doucement vers la droite pour les suivre.
pub fn sys_relax_shards(sim: &mut SimWorld, metrics: &HashMap<u32, ShardMetrics>, lerp_factor: f32) {
    let mut updates = Vec::new();
    for (entity, (pos, shard)) in sim.ecs.query_mut::<(&Position, &ShardInfo)>() {
        if let Some(m) = metrics.get(&shard.id) {
            if m.count > 0 {
                let cx = m.centroid.x / (m.count as f32);
                let cy = m.centroid.y / (m.count as f32);
                let dx = cx - pos.x;
                let dy = cy - pos.y;
                if dx * dx + dy * dy > 1.0 { // Marge morte pour éviter les micro-vibrations
                    updates.push((entity, *pos, pos.x + dx * lerp_factor, pos.y + dy * lerp_factor));
                }
            }
        }
    }
    for (entity, old_pos, nx, ny) in updates {
        sim.move_shard(entity, &old_pos, nx, ny);
    }
}

pub fn sys_dynamic_sharding(sim: &mut SimWorld, metrics: &HashMap<u32, ShardMetrics>) {
    let mut to_remove = Vec::new();
    let mut new_spawns = Vec::new();

    for (entity, (pos, shard)) in sim.ecs.query_mut::<(&Position, &ShardInfo)>() {
        if let Some(m) = metrics.get(&shard.id) {
            // Un shard surpeuplé est détruit pour laisser place à deux nouveaux shards (Split)
            if m.count >= 5 && sim.current_tick > shard.spawn_tick + 60 {
                to_remove.push((entity, *pos, shard.id));

                // Utilisation des bornes mathématiques (pas de Vec alloué).
                // On détermine quel est l'axe d'étalement le plus long des joueurs
                let spread_x = m.max_bound.x - m.min_bound.x;
                let spread_y = m.max_bound.y - m.min_bound.y;

                // On coupe perpendiculairement à l'axe de plus grand étalement
                let (p1, p2) = if spread_x > spread_y {
                    (
                        Point2D { x: m.min_bound.x, y: pos.y },
                        Point2D { x: m.max_bound.x, y: pos.y }
                    )
                } else {
                    (
                        Point2D { x: pos.x, y: m.min_bound.y },
                        Point2D { x: pos.x, y: m.max_bound.y }
                    )
                };
                new_spawns.push(p1); new_spawns.push(p2);
            }
        }
    }

    let mut removed_ids = Vec::new();
    for (ent, pos, id) in to_remove {
        sim.despawn_shard(ent, &pos);
        removed_ids.push(id);
        sim.stats_splits += 1;
    }
    for p in new_spawns {
        sim.spawn_shard(p.x, p.y, sim.next_shard_id, sim.current_tick);
        sim.next_shard_id += 1;
    }

    // Réaffectation d'urgence pour les joueurs dont le shard vient de mourir (Hard-fallback)
    if !removed_ids.is_empty() {
        let shards = get_shards_vec(&sim.ecs);
        for (_, (pos, player)) in sim.ecs.query_mut::<(&Position, &mut PlayerInfo)>() {
            if removed_ids.contains(&player.current_shard_id) {
                player.current_shard_id = find_nearest_shard_id(&Point2D{x: pos.x, y: pos.y}, &shards);
            }
        }
    }
}

pub fn sys_merge_shards(sim: &mut SimWorld, metrics: &HashMap<u32, ShardMetrics>) {
    let shards = get_shards_vec(&sim.ecs);
    if shards.len() <= 1 { return; }

    let mut best_pair: Option<(usize, usize)> = None;
    let mut min_dist_sq = f32::MAX;
    let max_merge_dist_sq = 400.0 * 400.0;

    for i in 0..shards.len() {
        for j in (i + 1)..shards.len() {
            if sim.current_tick < shards[i].spawn_tick + 60 || sim.current_tick < shards[j].spawn_tick + 60 { continue; }
            let count_i = metrics.get(&shards[i].id).map(|m| m.count).unwrap_or(0);
            let count_j = metrics.get(&shards[j].id).map(|m| m.count).unwrap_or(0);

            // Règle métier : On ne fusionne que si la population combinée est faible
            if count_i + count_j <= 2 {
                let dist_sq = shards[i].pos.distance_sq(&shards[j].pos);
                if dist_sq < min_dist_sq && dist_sq < max_merge_dist_sq {
                    min_dist_sq = dist_sq;
                    best_pair = Some((i, j));
                }
            }
        }
    }

    if let Some((i, j)) = best_pair {
        let new_x = (shards[i].pos.x + shards[j].pos.x) / 2.0;
        let new_y = (shards[i].pos.y + shards[j].pos.y) / 2.0;
        let id_i = shards[i].id; let id_j = shards[j].id;

        let mut to_despawn = Vec::new();
        for (entity, (pos, shard)) in sim.ecs.query_mut::<(&Position, &ShardInfo)>() {
            if shard.id == id_i || shard.id == id_j { to_despawn.push((entity, *pos)); }
        }
        for (ent, pos) in to_despawn { sim.despawn_shard(ent, &pos); }

        sim.spawn_shard(new_x, new_y, sim.next_shard_id, sim.current_tick);
        sim.next_shard_id += 1;
        sim.stats_merges += 1;

        let new_shards_list = get_shards_vec(&sim.ecs);
        for (_, (pos, player)) in sim.ecs.query_mut::<(&Position, &mut PlayerInfo)>() {
            if player.current_shard_id == id_i || player.current_shard_id == id_j {
                player.current_shard_id = find_nearest_shard_id(&Point2D{x: pos.x, y: pos.y}, &new_shards_list);
            }
        }
    }
}

pub fn sys_evaluate_handoffs(sim: &mut SimWorld, voronoi_data: &HashMap<u32, VoronoiCellData>, hysteresis: f32) {
    let shards = get_shards_vec(&sim.ecs);
    let mut handoffs = 0;

    // Pour une version multithreadée à 100k joueurs, on remplacerait cette boucle
    // par hecs::World::query_mut().par_iter() (via la crate rayon)
    for (_, (pos, player)) in sim.ecs.query_mut::<(&Position, &mut PlayerInfo)>() {
        let p2d = Point2D{x: pos.x, y: pos.y};
        let new_shard = evaluate_handoff(&p2d, player.current_shard_id, &shards, voronoi_data, hysteresis);
        if new_shard != player.current_shard_id {
            player.current_shard_id = new_shard;
            handoffs += 1;
        }
    }
    sim.stats_handoffs += handoffs;
}