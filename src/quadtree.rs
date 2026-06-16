// src/quadtree.rs
use crate::engine::{PlayerData, PlayerKey};
use crate::voronoi::Point2D;
use slotmap::SlotMap;

#[derive(Debug, Clone, Copy)]
pub struct QuadRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl QuadRect {
    pub fn contains(&self, p: &Point2D) -> bool {
        p.x >= self.x && p.x <= self.x + self.w &&
            p.y >= self.y && p.y <= self.y + self.h
    }
}

pub struct QuadNode {
    pub id: u32,
    pub rect: QuadRect,
    pub spawn_tick: u64,
    pub children: Option<Box<[QuadNode; 4]>>,
}

impl QuadNode {
    pub fn new(id: u32, rect: QuadRect, spawn_tick: u64) -> Self {
        Self { id, rect, spawn_tick, children: None }
    }

    pub fn find_leaf_id(&self, pos: &Point2D) -> u32 {
        if let Some(children) = &self.children {
            for child in children.iter() {
                if child.rect.contains(pos) {
                    return child.find_leaf_id(pos);
                }
            }
        }
        self.id
    }

    pub fn collect_leaves<'a>(&'a self, leaves: &mut Vec<&'a QuadNode>) {
        if let Some(children) = &self.children {
            for child in children.iter() {
                child.collect_leaves(leaves);
            }
        } else {
            leaves.push(self);
        }
    }

    // Adapté pour lire directement depuis la SlotMap de l'Orchestrateur
    fn count_players_in_rect(rect: &QuadRect, players: &SlotMap<PlayerKey, PlayerData>) -> usize {
        players.values().filter(|p| rect.contains(&p.pos)).count()
    }

    pub fn update(&mut self, players: &SlotMap<PlayerKey, PlayerData>, next_shard_id: &mut u32, current_tick: u64) -> bool {
        let mut changed = false;

        if let Some(children) = &mut self.children {
            for child in children.iter_mut() {
                changed |= child.update(players, next_shard_id, current_tick);
            }

            let all_leaves = children.iter().all(|c| c.children.is_none());
            // Délai de grâce adapté pour 2 Hz (ex: 10 ticks = 5 secondes)
            let cooldown_ok = children.iter().all(|c| current_tick > c.spawn_tick + 10);

            if all_leaves && cooldown_ok {
                let total_pop: usize = children.iter()
                    .map(|c| Self::count_players_in_rect(&c.rect, players))
                    .sum();

                if total_pop <= 2 {
                    self.children = None;
                    self.spawn_tick = current_tick;
                    return true;
                }
            }
        } else {
            if current_tick > self.spawn_tick + 10 {
                let pop = Self::count_players_in_rect(&self.rect, players);

                if pop >= 5 {
                    let w = self.rect.w / 2.0;
                    let h = self.rect.h / 2.0;
                    let x = self.rect.x;
                    let y = self.rect.y;

                    self.children = Some(Box::new([
                        QuadNode::new(*next_shard_id, QuadRect { x, y, w, h }, current_tick),
                        QuadNode::new(*next_shard_id + 1, QuadRect { x: x + w, y, w, h }, current_tick),
                        QuadNode::new(*next_shard_id + 2, QuadRect { x, y: y + h, w, h }, current_tick),
                        QuadNode::new(*next_shard_id + 3, QuadRect { x: x + w, y: y + h, w, h }, current_tick),
                    ]));

                    *next_shard_id += 4;
                    self.spawn_tick = current_tick;
                    return true;
                }
            }
        }

        changed
    }
}