//! Sparse, unbounded, chunked grid.
//!
//! The grid is divided into 32x32 chunks stored in a hash map.  A simulation
//! step therefore performs exactly one hash lookup (plus one when a new chunk
//! is created), which keeps the inner loop cheap while allowing the ant to
//! travel arbitrarily far without wrapping or clamping.

use crate::genome::MAX_COLORS;
use std::collections::HashMap;

pub const CHUNK: i32 = 32;
pub const CHUNK_AREA: usize = (CHUNK as usize) * (CHUNK as usize);
const WORDS: usize = CHUNK_AREA / 64;

#[derive(Clone)]
pub struct Chunk {
    pub cells: [u8; CHUNK_AREA],
    /// One bit per cell: was the cell ever written?
    pub seen: [u64; WORDS],
}

impl Chunk {
    fn new() -> Box<Chunk> {
        Box::new(Chunk {
            cells: [0; CHUNK_AREA],
            seen: [0; WORDS],
        })
    }

    #[inline]
    fn was_seen(&self, local: usize) -> bool {
        (self.seen[local / 64] >> (local % 64)) & 1 != 0
    }

    #[inline]
    fn mark_seen(&mut self, local: usize) {
        self.seen[local / 64] |= 1u64 << (local % 64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BBox {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl BBox {
    pub fn width(&self) -> u64 {
        (self.max_x as i64 - self.min_x as i64 + 1).max(0) as u64
    }
    pub fn height(&self) -> u64 {
        (self.max_y as i64 - self.min_y as i64 + 1).max(0) as u64
    }
    pub fn area(&self) -> u64 {
        self.width() * self.height()
    }
    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_x as f64 + self.max_x as f64) / 2.0,
            (self.min_y as f64 + self.max_y as f64) / 2.0,
        )
    }
}

#[derive(Clone)]
pub struct Grid {
    chunks: HashMap<(i32, i32), Box<Chunk>>,
    /// Cells written for the first time.
    pub new_cells: u64,
    /// Total write operations (= number of simulation steps).
    pub writes: u64,
    /// Number of cells currently holding each color.
    pub color_counts: [u64; MAX_COLORS],
    bbox: Option<BBox>,
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}

impl Grid {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            new_cells: 0,
            writes: 0,
            color_counts: [0; MAX_COLORS],
            bbox: None,
        }
    }

    #[inline]
    fn key(x: i32, y: i32) -> ((i32, i32), usize) {
        let cx = x.div_euclid(CHUNK);
        let cy = y.div_euclid(CHUNK);
        let lx = x.rem_euclid(CHUNK) as usize;
        let ly = y.rem_euclid(CHUNK) as usize;
        ((cx, cy), ly * CHUNK as usize + lx)
    }

    #[inline]
    pub fn read(&self, x: i32, y: i32) -> u8 {
        let (key, local) = Self::key(x, y);
        self.chunks.get(&key).map(|c| c.cells[local]).unwrap_or(0)
    }

    #[inline]
    pub fn is_seen(&self, x: i32, y: i32) -> bool {
        let (key, local) = Self::key(x, y);
        self.chunks
            .get(&key)
            .map(|c| c.was_seen(local))
            .unwrap_or(false)
    }

    /// Write `color` at `(x, y)`.  Returns `true` when the cell was visited for
    /// the first time (and therefore grew the bounding box).
    #[inline]
    pub fn write(&mut self, x: i32, y: i32, color: u8) -> bool {
        let (key, local) = Self::key(x, y);
        let chunk = self.chunks.entry(key).or_insert_with(Chunk::new);
        self.writes += 1;
        let old = chunk.cells[local];
        let first = !chunk.was_seen(local);
        if first {
            chunk.mark_seen(local);
            self.new_cells += 1;
            self.color_counts[color as usize] += 1;
            self.bbox = Some(match self.bbox {
                None => BBox {
                    min_x: x,
                    min_y: y,
                    max_x: x,
                    max_y: y,
                },
                Some(b) => BBox {
                    min_x: b.min_x.min(x),
                    min_y: b.min_y.min(y),
                    max_x: b.max_x.max(x),
                    max_y: b.max_y.max(y),
                },
            });
        } else if old != color {
            self.color_counts[old as usize] -= 1;
            self.color_counts[color as usize] += 1;
        }
        chunk.cells[local] = color;
        first
    }

    pub fn bbox(&self) -> Option<BBox> {
        self.bbox
    }

    pub fn colors_used(&self, colors: u8) -> usize {
        self.color_counts[..colors as usize]
            .iter()
            .filter(|&&c| c > 0)
            .count()
    }

    pub fn revisits(&self) -> u64 {
        self.writes.saturating_sub(self.new_cells)
    }

    /// Compare the color configuration of this (older) pattern with a later
    /// one translated by `(dx, dy)`, restricted to a square window around
    /// `center`.
    ///
    /// Returns `(matched, total)` over the cells that were already visited in
    /// the older pattern.  Unvisited cells are treated as color 0, exactly as
    /// the ant sees them.  A highway is a pattern for which every old cell in
    /// the travelling region reappears, with the same color, displaced by the
    /// drift accumulated over one period; new cells at the frontier and the
    /// frozen transient near the origin are excluded by the window.
    pub fn embedding_match(
        &self,
        later: &Grid,
        dx: i32,
        dy: i32,
        center: (i32, i32),
        radius: i32,
    ) -> (u64, u64) {
        let (mut matched, mut total) = (0u64, 0u64);
        for (&(cx, cy), chunk) in &self.chunks {
            for local in 0..CHUNK_AREA {
                if !chunk.was_seen(local) {
                    continue;
                }
                let x = cx * CHUNK + (local % CHUNK as usize) as i32;
                let y = cy * CHUNK + (local / CHUNK as usize) as i32;
                if (x - center.0).abs() > radius || (y - center.1).abs() > radius {
                    continue;
                }
                total += 1;
                if later.read(x + dx, y + dy) == chunk.cells[local] {
                    matched += 1;
                }
            }
        }
        (matched, total)
    }

    /// Approximate number of distinct 3x3 neighborhoods in the visiting
    /// pattern, using a 64k-bit Bloom filter and deterministic sampling.
    ///
    /// This is a cheap proxy for morphological complexity: a solid band
    /// produces a handful of distinct neighborhoods, while a structured
    /// pattern produces hundreds.  Returns `(distinct_estimate, samples)`.
    pub fn neighborhood_diversity(&self, stride: usize) -> (u64, u64) {
        const BITS: usize = 1 << 16;
        let mut bloom = vec![0u64; BITS / 64];
        let stride = stride.max(1);
        let mut seen = 0usize;
        let mut samples = 0u64;
        let mut keys: Vec<(i32, i32)> = self.chunks.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let (cx, cy) = key;
            let chunk = &self.chunks[&key];
            for local in 0..CHUNK_AREA {
                if !chunk.was_seen(local) {
                    continue;
                }
                if seen % stride != 0 {
                    seen += 1;
                    continue;
                }
                seen += 1;
                samples += 1;
                let x = cx * CHUNK + (local % CHUNK as usize) as i32;
                let y = cy * CHUNK + (local / CHUNK as usize) as i32;
                let mut code: u32 = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        code = (code << 3) | self.read(x + dx, y + dy) as u32;
                    }
                }
                let mut h = code.wrapping_mul(0x9E37_79B1);
                h ^= h >> 15;
                let bit = (h as usize) & (BITS - 1);
                bloom[bit / 64] |= 1u64 << (bit % 64);
            }
        }
        let distinct: u64 = bloom.iter().map(|w| w.count_ones() as u64).sum();
        (distinct, samples)
    }

    /// Deterministic 2D hash of the visited pattern, used as a cheap identity
    /// for patterns in tests and gallery reports.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pattern_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut keys: Vec<(i32, i32)> = self.chunks.keys().copied().collect();
        keys.sort_unstable();
        for (cx, cy) in keys {
            let chunk = &self.chunks[&(cx, cy)];
            for local in 0..CHUNK_AREA {
                if chunk.was_seen(local) {
                    h ^= chunk.cells[local] as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                    h ^= (cx as u64).rotate_left(17) ^ (cy as u64).rotate_left(31);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_and_bbox() {
        let mut g = Grid::new();
        assert_eq!(g.read(0, 0), 0);
        assert!(g.write(0, 0, 1));
        assert!(!g.write(0, 0, 2));
        assert_eq!(g.read(0, 0), 2);
        assert_eq!(g.new_cells, 1);
        assert_eq!(g.writes, 2);
        assert_eq!(g.color_counts[1], 0);
        assert_eq!(g.color_counts[2], 1);
        assert_eq!(
            g.bbox().unwrap(),
            BBox {
                min_x: 0,
                min_y: 0,
                max_x: 0,
                max_y: 0
            }
        );
        assert!(g.write(-1, 5, 0));
        assert_eq!(g.bbox().unwrap().min_x, -1);
        assert_eq!(g.bbox().unwrap().max_y, 5);
    }

    #[test]
    fn negative_coordinates_and_chunk_boundaries() {
        let mut g = Grid::new();
        for x in -70..70 {
            for y in -70..70 {
                g.write(x, y, ((x + y).rem_euclid(3)) as u8);
            }
        }
        for x in [-64, -33, -32, -31, 0, 31, 32, 63] {
            for y in [-64, -32, 0, 31, 32] {
                assert!(g.is_seen(x, y));
            }
        }
        assert_eq!(g.new_cells, 140 * 140);
    }

    #[test]
    fn translation_detection() {
        let mut old = Grid::new();
        for i in 0..10 {
            old.write(i, 0, 1);
        }
        let mut later = Grid::new();
        for i in 0..20 {
            later.write(i, 0, 1);
        }
        assert_eq!(old.embedding_match(&later, 1, 0, (9, 0), 100), (10, 10));
        assert_eq!(old.embedding_match(&later, 1, 1, (9, 0), 100), (0, 10));
        assert_eq!(old.embedding_match(&later, 20, 0, (9, 0), 100), (0, 10));
    }
}
