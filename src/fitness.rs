//! Multi-objective fitness, collapsed into a weighted scalar for selection.
//!
//! The four components capture different, partially conflicting, notions of
//! "interesting" collective behaviour:
//!
//! * **mobility**    -- directed transport: net displacement per step.  Large
//!   values are produced by highways, gliders and conveyor belts.
//! * **compactness** -- fraction of the bounding box that the ant visits.
//!   Large values indicate dense constructions instead of a thin trail lost in
//!   empty space.
//! * **complexity**  -- diversity of 3x3 neighborhoods in the visited pattern
//!   (estimated with a Bloom filter).  Solid bands score near zero; structured,
//!   ramified or multiscale patterns score high.
//! * **colorfulness** -- fraction of the available cell colors that are used.
//!   A weak pressure against degenerate single-color rules.
//!
//! All components live in `[0, 1]`, which makes the weighted sum easy to
//! interpret and to ablate.

use crate::genome::Genome;
use crate::sim::{simulate, SimOutcome, SimParams};

#[derive(Clone, Copy, Debug)]
pub struct Weights {
    pub mobility: f64,
    pub compactness: f64,
    pub complexity: f64,
    pub colorfulness: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            mobility: 0.50,
            compactness: 0.25,
            complexity: 0.15,
            colorfulness: 0.10,
        }
    }
}

impl Weights {
    pub fn normalized(mut self) -> Self {
        let s = self.mobility + self.compactness + self.complexity + self.colorfulness;
        if s > 0.0 {
            self.mobility /= s;
            self.compactness /= s;
            self.complexity /= s;
            self.colorfulness /= s;
        }
        self
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 4 {
            return Err("weights must be 'mobility,compactness,complexity,colorfulness'".into());
        }
        let mut v = [0.0; 4];
        for (i, p) in parts.iter().enumerate() {
            v[i] = p
                .trim()
                .parse::<f64>()
                .map_err(|e| format!("bad weight {p:?}: {e}"))?;
            if !(0.0..=10.0).contains(&v[i]) {
                return Err("weights must be in [0, 10]".into());
            }
        }
        Ok(Self {
            mobility: v[0],
            compactness: v[1],
            complexity: v[2],
            colorfulness: v[3],
        }
        .normalized())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fitness {
    pub total: f64,
    pub mobility: f64,
    pub compactness: f64,
    pub complexity: f64,
    pub colorfulness: f64,
    pub displacement: f64,
    pub new_cells: u64,
    pub revisits: u64,
    pub bbox_area: u64,
    pub colors_used: usize,
    pub neighborhoods: u64,
}

impl Fitness {
    pub fn score(outcome: &SimOutcome, colors: u8, weights: &Weights) -> Self {
        let steps = outcome.steps.max(1) as f64;
        let displacement = outcome.displacement();
        let mobility = (displacement / steps).clamp(0.0, 1.0);
        let bbox_area = outcome.bbox_area().max(1);
        let compactness = (outcome.new_cells() as f64 / bbox_area as f64).clamp(0.0, 1.0);
        let (neighborhoods, _samples) = outcome.grid.neighborhood_diversity(4);
        // Absolute count of distinct 3x3 neighborhoods, saturating at 512.
        // Absolute (rather than relative) diversity avoids rewarding tiny
        // patterns that happen to contain a handful of local motifs.
        let complexity = (neighborhoods as f64 / 512.0).clamp(0.0, 1.0);
        let colors_used = outcome.colors_used();
        let colorfulness = (colors_used as f64 / colors.max(1) as f64).clamp(0.0, 1.0);
        let total = weights.mobility * mobility
            + weights.compactness * compactness
            + weights.complexity * complexity
            + weights.colorfulness * colorfulness;
        // A rule that does not move and does not build anything is worthless.
        let total = if outcome.new_cells() < 2 { 0.0 } else { total };
        Self {
            total,
            mobility,
            compactness,
            complexity,
            colorfulness,
            displacement,
            new_cells: outcome.new_cells(),
            revisits: outcome.revisits(),
            bbox_area: outcome.bbox_area(),
            colors_used,
            neighborhoods,
        }
    }
}

pub fn evaluate(genome: &Genome, params: &SimParams, weights: &Weights) -> Fitness {
    let outcome = simulate(genome, params);
    Fitness::score(&outcome, genome.colors, weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(steps: u32) -> SimParams {
        SimParams {
            steps,
            ..Default::default()
        }
    }

    #[test]
    fn classic_langton_is_not_degenerate() {
        let f = evaluate(&Genome::langton(), &params(10_000), &Weights::default());
        assert!(f.total > 0.1 && f.total < 1.0, "{f:?}");
        assert!(f.new_cells > 100);
        assert_eq!(f.colors_used, 2);
        assert!(f.complexity > 0.3, "{f:?}");
    }

    #[test]
    fn mobility_is_bounded() {
        let mut g = Genome::new(1, 1);
        // Always turn right: after four steps the ant returns to the origin.
        g.set(
            0,
            0,
            crate::genome::Rule {
                write: 0,
                turn: crate::genome::Turn::Right,
                next: 0,
            },
        );
        let f = evaluate(&g, &params(100), &Weights::default());
        assert!(f.mobility <= 1.0);
        assert!(f.total <= 1.0);
    }

    #[test]
    fn weights_normalize() {
        let w = Weights {
            mobility: 1.0,
            compactness: 1.0,
            complexity: 1.0,
            colorfulness: 1.0,
        }
        .normalized();
        assert!((w.mobility - 0.25).abs() < 1e-12);
        assert!((w.colorfulness - 0.25).abs() < 1e-12);
    }

    #[test]
    fn weights_parse() {
        let w = Weights::parse("0.5,0.25,0.15,0.1").unwrap();
        assert!((w.mobility - 0.5).abs() < 1e-12);
        assert!(Weights::parse("1,2").is_err());
        assert!(Weights::parse("a,b,c,d").is_err());
    }
}
