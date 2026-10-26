//! Island-model genetic algorithm (generational, elitist, tournament selection,
//! uniform crossover, per-entry mutation).

use crate::fitness::{evaluate, Fitness, Weights};
use crate::genome::{Genome, Rule, Turn};
use crate::rng::Rng;
use crate::sim::SimParams;
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct GaConfig {
    pub pop: usize,
    pub gens: u32,
    pub steps: u32,
    pub states: u8,
    pub colors: u8,
    pub tournament: usize,
    pub elite: usize,
    pub mutation_rate: f64,
    pub reset_rate: f64,
    pub dedupe: bool,
    pub seed: u64,
    pub threads: usize,
    pub weights: Weights,
    pub classic_seed: bool,
}

impl Default for GaConfig {
    fn default() -> Self {
        Self {
            pop: 64,
            gens: 100,
            steps: 20_000,
            states: 2,
            colors: 2,
            tournament: 3,
            elite: 2,
            mutation_rate: 0.03,
            reset_rate: 0.01,
            dedupe: true,
            seed: 1,
            threads: 1,
            weights: Weights::default(),
            classic_seed: true,
        }
    }
}

#[derive(Clone)]
pub struct Individual {
    pub genome: Genome,
    pub fitness: Fitness,
    pub evaluated: bool,
}

impl Individual {
    fn new(genome: Genome) -> Self {
        Self {
            genome,
            fitness: Fitness::default(),
            evaluated: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GenStats {
    pub generation: u32,
    pub best: f64,
    pub mean: f64,
    pub median: f64,
    pub worst: f64,
    pub stddev: f64,
    pub diversity: f64,
    pub behavioral: f64,
    pub evaluations: u64,
    pub best_mobility: f64,
    pub best_compactness: f64,
    pub best_complexity: f64,
    pub best_colorfulness: f64,
}

pub struct Island {
    pub cfg: GaConfig,
    pub population: Vec<Individual>,
    pub generation: u32,
    pub evaluations: u64,
    pub best_ever: Option<(u32, f64, Genome)>,
    rng: Rng,
}

impl Island {
    pub fn new(cfg: GaConfig) -> Self {
        assert!(cfg.pop >= 4);
        assert!((0.0..=1.0).contains(&cfg.mutation_rate));
        let mut rng = Rng::new(cfg.seed);
        let mut population = Vec::with_capacity(cfg.pop);
        for i in 0..cfg.pop {
            let mut genome = Genome::random(cfg.states, cfg.colors, &mut rng);
            if cfg.classic_seed && cfg.states >= 2 && cfg.colors >= 2 {
                if i == 0 {
                    seed_classic(&mut genome);
                } else if i < cfg.pop / 8 {
                    seed_classic(&mut genome);
                    genome.mutate(&mut rng, 0.15);
                }
            }
            population.push(Individual::new(genome));
        }
        Self {
            cfg,
            population,
            generation: 0,
            evaluations: 0,
            best_ever: None,
            rng,
        }
    }

    /// Evaluate every individual whose fitness is stale.  Uses up to
    /// `cfg.threads` OS threads (scoped, no external dependencies).
    pub fn evaluate_pending(&mut self) {
        let pending = self.population.iter().filter(|i| !i.evaluated).count();
        if pending == 0 {
            return;
        }
        let params = SimParams {
            steps: self.cfg.steps,
            ..Default::default()
        };
        let weights = self.cfg.weights;
        let threads = self
            .cfg
            .threads
            .max(1)
            .min(pending)
            .min(self.population.len());
        if threads <= 1 {
            for ind in &mut self.population {
                if !ind.evaluated {
                    ind.fitness = evaluate(&ind.genome, &params, &weights);
                    ind.evaluated = true;
                }
            }
        } else {
            let chunk = (self.population.len() + threads - 1) / threads;
            std::thread::scope(|scope| {
                for part in self.population.chunks_mut(chunk) {
                    scope.spawn(move || {
                        for ind in part.iter_mut() {
                            if !ind.evaluated {
                                ind.fitness = evaluate(&ind.genome, &params, &weights);
                                ind.evaluated = true;
                            }
                        }
                    });
                }
            });
        }
        self.evaluations += pending as u64;
    }

    pub fn best(&self) -> Option<&Individual> {
        self.population
            .iter()
            .filter(|i| i.evaluated)
            .max_by(|a, b| {
                a.fitness
                    .total
                    .partial_cmp(&b.fitness.total)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Clone of the best individual's genome (used by the deterministic tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn best_genome(&self) -> Option<Genome> {
        self.best().map(|i| i.genome.clone())
    }

    fn tournament(&mut self, order: &[usize]) -> Genome {
        let k = self.cfg.tournament.max(1);
        let mut best = order[self.rng.below(order.len())];
        for _ in 1..k {
            let candidate = order[self.rng.below(order.len())];
            if self.population[candidate].fitness.total > self.population[best].fitness.total {
                best = candidate;
            }
        }
        self.population[best].genome.clone()
    }

    fn snapshot(&self, evaluations_before: u64) -> GenStats {
        let mut fits: Vec<f64> = self.population.iter().map(|i| i.fitness.total).collect();
        fits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = fits.len();
        let mean = fits.iter().sum::<f64>() / n as f64;
        let var = fits.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / n as f64;
        let best_ind = self.best().expect("population is never empty");
        let unique: HashSet<u64> = self
            .population
            .iter()
            .map(|i| i.genome.fingerprint())
            .collect();
        let behaviors: HashSet<(i64, i64, i64, i64)> = self
            .population
            .iter()
            .map(|i| {
                let f = &i.fitness;
                (
                    (f.mobility * 1000.0).round() as i64,
                    (f.compactness * 1000.0).round() as i64,
                    (f.complexity * 1000.0).round() as i64,
                    (f.colorfulness * 1000.0).round() as i64,
                )
            })
            .collect();
        GenStats {
            generation: self.generation,
            best: fits[n - 1],
            mean,
            median: if n % 2 == 0 {
                (fits[n / 2 - 1] + fits[n / 2]) / 2.0
            } else {
                fits[n / 2]
            },
            worst: fits[0],
            stddev: var.sqrt(),
            diversity: unique.len() as f64 / n as f64,
            behavioral: behaviors.len() as f64 / n as f64,
            evaluations: self.evaluations - evaluations_before,
            best_mobility: best_ind.fitness.mobility,
            best_compactness: best_ind.fitness.compactness,
            best_complexity: best_ind.fitness.complexity,
            best_colorfulness: best_ind.fitness.colorfulness,
        }
    }

    /// Evaluate the current generation, log statistics, and breed the next one.
    pub fn step(&mut self) -> GenStats {
        let before = self.evaluations;
        self.evaluate_pending();
        let stats = self.snapshot(before);
        if stats.best > self.best_ever.as_ref().map(|b| b.1).unwrap_or(f64::MIN) {
            if let Some(ind) = self.best() {
                self.best_ever = Some((self.generation, ind.fitness.total, ind.genome.clone()));
            }
        }

        let pop = self.cfg.pop;
        let mut order: Vec<usize> = (0..pop).collect();
        order.sort_unstable_by(|&a, &b| {
            self.population[b]
                .fitness
                .total
                .partial_cmp(&self.population[a].fitness.total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let elite_n = self.cfg.elite.min(pop.saturating_sub(1));
        let mut next: Vec<Individual> = order[..elite_n]
            .iter()
            .map(|&i| self.population[i].clone())
            .collect();
        let mut seen: HashSet<u64> = next.iter().map(|i| i.genome.fingerprint()).collect();

        while next.len() < pop {
            let a = self.tournament(&order);
            let b = self.tournament(&order);
            let mut genome = Genome::crossover(&a, &b, &mut self.rng);
            genome.mutate(&mut self.rng, self.cfg.mutation_rate);
            if self.rng.chance(self.cfg.reset_rate) {
                genome = Genome::random(self.cfg.states, self.cfg.colors, &mut self.rng);
            }
            if self.cfg.dedupe {
                let mut tries = 0;
                while tries < 4 && seen.contains(&genome.fingerprint()) {
                    genome.mutate(&mut self.rng, 0.08);
                    tries += 1;
                }
                if seen.contains(&genome.fingerprint()) {
                    genome = Genome::random(self.cfg.states, self.cfg.colors, &mut self.rng);
                }
            }
            seen.insert(genome.fingerprint());
            next.push(Individual::new(genome));
        }
        self.population = next;
        self.generation += 1;
        stats
    }

    /// Replace the worst individuals by immigrants, avoiding duplicates.
    /// Returns the number of genomes actually accepted.
    pub fn inject(&mut self, immigrants: &[Genome]) -> usize {
        self.evaluate_pending();
        if immigrants.is_empty() {
            return 0;
        }
        let pop = self.cfg.pop;
        let mut order: Vec<usize> = (0..pop).collect();
        order.sort_unstable_by(|&a, &b| {
            self.population[a]
                .fitness
                .total
                .partial_cmp(&self.population[b].fitness.total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let protected = self.cfg.elite.min(pop);
        let mut accepted = 0;
        let mut existing: HashSet<u64> = self
            .population
            .iter()
            .map(|i| i.genome.fingerprint())
            .collect();
        for genome in immigrants {
            if accepted >= pop.saturating_sub(protected) {
                break;
            }
            if existing.contains(&genome.fingerprint()) {
                continue;
            }
            let victim = order[accepted];
            self.population[victim] = Individual::new(genome.clone());
            existing.insert(genome.fingerprint());
            accepted += 1;
        }
        accepted
    }
}

fn seed_classic(genome: &mut Genome) {
    let next = if genome.states >= 2 { 0 } else { 0 };
    genome.set(
        0,
        0,
        Rule {
            write: if genome.colors >= 2 { 1 } else { 0 },
            turn: Turn::Right,
            next,
        },
    );
    genome.set(
        0,
        1,
        Rule {
            write: 0,
            turn: Turn::Left,
            next,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_cfg(seed: u64) -> GaConfig {
        GaConfig {
            pop: 12,
            steps: 1_500,
            seed,
            threads: 2,
            ..Default::default()
        }
    }

    #[test]
    fn evaluation_and_generation_advance() {
        let mut isl = Island::new(small_cfg(1));
        let s = isl.step();
        assert_eq!(s.generation, 0);
        assert_eq!(s.evaluations, 12);
        assert!(s.best >= s.mean && s.mean >= s.worst - 1e-12);
        assert!(s.diversity > 0.0);
        assert!(isl.best_ever.is_some());
        let s2 = isl.step();
        assert_eq!(s2.generation, 1);
        // Elites are cached, so only the new offspring are evaluated again.
        assert!(s2.evaluations <= 12 && s2.evaluations >= 1);
        assert_eq!(isl.evaluations, 12 + s2.evaluations);
    }

    #[test]
    fn injection_keeps_population_size_and_fitness() {
        let mut isl = Island::new(small_cfg(2));
        isl.step();
        let outsider = Genome::random(2, 2, &mut Rng::new(99));
        let n = isl.inject(&[outsider]);
        assert!(n <= 1);
        assert_eq!(isl.population.len(), isl.cfg.pop);
        isl.evaluate_pending();
        assert!(isl.population.iter().all(|i| i.evaluated));
    }

    #[test]
    fn deterministic_given_seed() {
        let mut a = Island::new(small_cfg(7));
        let mut b = Island::new(small_cfg(7));
        for _ in 0..3 {
            let sa = a.step();
            let sb = b.step();
            assert!((sa.best - sb.best).abs() < 1e-12);
        }
        assert_eq!(a.best_genome().unwrap(), b.best_genome().unwrap());
    }
}
