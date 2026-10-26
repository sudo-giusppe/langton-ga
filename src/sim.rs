//! Deterministic turmite simulator and trajectory analysis.

use crate::genome::Genome;
use crate::grid::{BBox, Grid};

/// North, east, south, west.  Screen coordinates: x grows right, y grows down.
pub const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
pub const DIR_NAMES: [&str; 4] = ["N", "E", "S", "W"];

#[derive(Clone, Copy, Debug)]
pub struct SimParams {
    pub steps: u32,
    pub start_x: i32,
    pub start_y: i32,
    pub dir: u8,
    pub state: u8,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            steps: 10_000,
            start_x: 0,
            start_y: 0,
            dir: 0,
            state: 0,
        }
    }
}

pub struct SimOutcome {
    pub grid: Grid,
    pub steps: u32,
    pub start: (i32, i32),
    pub pos: (i32, i32),
    pub dir: u8,
    pub state: u8,
    pub bbox: Option<BBox>,
}

impl SimOutcome {
    pub fn displacement(&self) -> f64 {
        let dx = (self.pos.0 - self.start.0) as f64;
        let dy = (self.pos.1 - self.start.1) as f64;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn new_cells(&self) -> u64 {
        self.grid.new_cells
    }

    pub fn revisits(&self) -> u64 {
        self.grid.revisits()
    }

    pub fn bbox_area(&self) -> u64 {
        self.bbox.map(|b| b.area()).unwrap_or(0)
    }

    pub fn colors_used(&self) -> usize {
        self.grid.colors_used(8)
    }
}

pub fn simulate(genome: &Genome, params: &SimParams) -> SimOutcome {
    simulate_with(genome, params, |_, _, _| {})
}

/// Simulate and invoke `observer(step, x, y)` after every step.  The callback
/// is used to trace trajectories without duplicating the inner loop.
pub fn simulate_with<F: FnMut(u32, i32, i32)>(
    genome: &Genome,
    params: &SimParams,
    mut observer: F,
) -> SimOutcome {
    let mut grid = Grid::new();
    let (mut x, mut y) = (params.start_x, params.start_y);
    let (mut dir, mut state) = (params.dir, params.state);
    for step in 0..params.steps {
        let color = grid.read(x, y);
        let rule = genome.rule(state, color);
        grid.write(x, y, rule.write);
        dir = ((dir as i8 + rule.turn.delta()).rem_euclid(4)) as u8;
        let (dx, dy) = DIRS[dir as usize];
        x += dx;
        y += dy;
        state = rule.next;
        debug_assert!((state as usize) < genome.states as usize);
        observer(step + 1, x, y);
    }
    SimOutcome {
        bbox: grid.bbox(),
        grid,
        steps: params.steps,
        start: (params.start_x, params.start_y),
        pos: (x, y),
        dir,
        state,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cycle {
    /// Number of steps between two identical local states.
    pub period: u64,
    /// Net displacement accumulated over one period.
    pub dx: i32,
    pub dy: i32,
    /// Step at which the recurrence was observed.
    pub at_step: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PatternCheck {
    pub matched: u64,
    pub total: u64,
}

impl PatternCheck {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.matched as f64 / self.total as f64
        }
    }
}

pub struct Analysis {
    pub outcome: SimOutcome,
    pub cycle: Option<Cycle>,
    pub check: Option<PatternCheck>,
}

/// Run `steps` and look for a highway.
///
/// A highway is a *spatio-temporal* periodic pattern: there is a period `p`
/// and a drift `(dx, dy)` such that (i) the sequence of local states
/// `(direction, internal state)` is periodic with period `p`, (ii) the
/// displacement over every block of `p` steps is constant, and (iii) the
/// pattern built at time `t - p` reappears, translated by `(dx, dy)`, at time
/// `t`.  Condition (iii) is what distinguishes a true highway from a
/// coincidental recurrence of the ant's local state, so we verify it by
/// re-simulating to `t - p` and checking every previously visited cell.
///
/// Candidate periods are found by testing the periodicity of the last `4p`
/// local states for every `p` up to `min(steps / 3, 4096)`.
pub fn analyze(genome: &Genome, params: &SimParams) -> Analysis {
    let mut grid = Grid::new();
    let (mut x, mut y) = (params.start_x, params.start_y);
    let (mut dir, mut state) = (params.dir, params.state);
    let steps = params.steps as usize;
    let mut pos: Vec<(i32, i32)> = Vec::with_capacity(steps + 1);
    let mut sym: Vec<u8> = Vec::with_capacity(steps + 1);
    pos.push((x, y));
    sym.push(dir * 8 + state);
    for _ in 0..params.steps {
        let color = grid.read(x, y);
        let rule = genome.rule(state, color);
        grid.write(x, y, rule.write);
        dir = ((dir as i8 + rule.turn.delta()).rem_euclid(4)) as u8;
        let (dx, dy) = DIRS[dir as usize];
        x += dx;
        y += dy;
        state = rule.next;
        pos.push((x, y));
        sym.push(dir * 8 + state);
    }
    let outcome = SimOutcome {
        bbox: grid.bbox(),
        grid,
        steps: params.steps,
        start: (params.start_x, params.start_y),
        pos: (x, y),
        dir,
        state,
    };

    let t = steps;
    let max_period = (steps / 3).min(4096);
    let mut candidates: Vec<(usize, i32, i32)> = Vec::new();
    for p in 8..=max_period {
        let drift = (pos[t].0 - pos[t - p].0, pos[t].1 - pos[t - p].1);
        if drift == (0, 0) {
            continue;
        }
        let window = (4 * p).min(t);
        let recent = (2 * p).min(t);
        let mut ok = true;
        for k in (t - window).max(p)..=t {
            if sym[k] != sym[k - p] {
                ok = false;
                break;
            }
            if k + recent >= t && (pos[k].0 - pos[k - p].0, pos[k].1 - pos[k - p].1) != drift {
                ok = false;
                break;
            }
        }
        if ok {
            candidates.push((p, drift.0, drift.1));
            if candidates.len() >= 16 {
                break;
            }
        }
    }

    let debug = std::env::var_os("LANGTON_DEBUG").is_some();
    let mut cycle = None;
    let mut check = None;
    for &(p, dx, dy) in candidates.iter().take(8) {
        let back = (t - p) as u32;
        let old = simulate(
            genome,
            &SimParams {
                steps: back,
                ..*params
            },
        );
        let (matched, total) = old
            .grid
            .embedding_match(&outcome.grid, dx, dy, old.pos, 128);
        let pc = PatternCheck { matched, total };
        let exact = total >= 50 && matched == total;
        if debug {
            eprintln!(
                "candidate period {} drift ({},{}) -> {}/{} ({:.4})",
                p,
                dx,
                dy,
                matched,
                total,
                pc.fraction()
            );
        }
        check = Some(pc);
        if exact {
            cycle = Some(Cycle {
                period: p as u64,
                dx,
                dy,
                at_step: t as u64,
            });
            break;
        }
    }
    Analysis {
        outcome,
        cycle,
        check,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::Turn;

    #[test]
    fn short_simulation_is_deterministic() {
        let p = SimParams {
            steps: 300,
            ..Default::default()
        };
        let a = simulate(&Genome::langton(), &p);
        let b = simulate(&Genome::langton(), &p);
        assert_eq!(a.pos, b.pos);
        assert_eq!(a.new_cells(), b.new_cells());
        assert_eq!(a.grid.pattern_hash(), b.grid.pattern_hash());
    }

    #[test]
    fn straight_rule_has_no_highway() {
        // Always turn left: a 4-step square, a genuine dynamic cycle with zero
        // drift, which is not a highway.
        let mut g = Genome::new(1, 1);
        g.set(
            0,
            0,
            crate::genome::Rule {
                write: 0,
                turn: Turn::Left,
                next: 0,
            },
        );
        let a = analyze(
            &g,
            &SimParams {
                steps: 1_000,
                ..Default::default()
            },
        );
        assert!(a.cycle.is_none());
    }
}
