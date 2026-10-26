//! Genotype of a generalized Langton ant (turmite).
//!
//! A genome is a complete rule table for a finite-state ant moving on a grid
//! whose cells can take `colors` values.  For every pair
//! `(state, cell_color)` the table stores
//!
//! * the color written back to the cell,
//! * the turn performed (`left`, `right` or `u-turn`),
//! * the next internal state of the ant.
//!
//! With `states = 2` and `colors = 2` the classic Langton rule is a single
//! point of this search space.

use crate::rng::Rng;
use std::fmt;

pub const MAX_COLORS: usize = 8;
pub const MAX_STATES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Turn {
    Left,
    Right,
    UTurn,
}

impl Turn {
    pub const ALL: [Turn; 3] = [Turn::Left, Turn::Right, Turn::UTurn];

    #[inline]
    pub fn delta(self) -> i8 {
        match self {
            Turn::Left => -1,
            Turn::Right => 1,
            Turn::UTurn => 2,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Turn::Left => 0,
            Turn::Right => 1,
            Turn::UTurn => 2,
        }
    }

    pub fn from_u8(v: u8) -> Option<Turn> {
        match v {
            0 => Some(Turn::Left),
            1 => Some(Turn::Right),
            2 => Some(Turn::UTurn),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    pub write: u8,
    pub turn: Turn,
    pub next: u8,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Genome {
    pub states: u8,
    pub colors: u8,
    pub rules: Vec<Rule>,
}

impl Genome {
    pub fn new(states: u8, colors: u8) -> Self {
        assert!(states >= 1 && states as usize <= MAX_STATES);
        assert!(colors >= 1 && colors as usize <= MAX_COLORS);
        Self {
            states,
            colors,
            rules: vec![
                Rule {
                    write: 0,
                    turn: Turn::Left,
                    next: 0
                };
                states as usize * colors as usize
            ],
        }
    }

    pub fn random(states: u8, colors: u8, rng: &mut Rng) -> Self {
        let mut g = Genome::new(states, colors);
        for r in &mut g.rules {
            r.write = rng.below(colors as usize) as u8;
            r.turn = rng.pick(&Turn::ALL);
            r.next = rng.below(states as usize) as u8;
        }
        g
    }

    /// The rule described by Christopher Langton (1986).  Only state 0 is
    /// reachable; state 1 is filled with a neutral rule.
    pub fn langton() -> Self {
        let mut g = Genome::new(2, 2);
        g.set(
            0,
            0,
            Rule {
                write: 1,
                turn: Turn::Right,
                next: 0,
            },
        );
        g.set(
            0,
            1,
            Rule {
                write: 0,
                turn: Turn::Left,
                next: 0,
            },
        );
        g
    }

    #[inline]
    pub fn index(&self, state: u8, color: u8) -> usize {
        state as usize * self.colors as usize + color as usize
    }

    #[inline]
    pub fn rule(&self, state: u8, color: u8) -> Rule {
        self.rules[self.index(state, color)]
    }

    pub fn set(&mut self, state: u8, color: u8, rule: Rule) {
        let i = self.index(state, color);
        self.rules[i] = rule;
    }

    pub fn mutate(&mut self, rng: &mut Rng, per_entry: f64) {
        for r in &mut self.rules {
            if rng.chance(per_entry) {
                r.write = rng.below(self.colors as usize) as u8;
            }
            if rng.chance(per_entry) {
                r.turn = rng.pick(&Turn::ALL);
            }
            if rng.chance(per_entry) {
                r.next = rng.below(self.states as usize) as u8;
            }
        }
    }

    /// Uniform (per rule entry) crossover.  The parents must have identical
    /// table shapes, which the island enforces.
    pub fn crossover(a: &Genome, b: &Genome, rng: &mut Rng) -> Genome {
        debug_assert_eq!(a.states, b.states);
        debug_assert_eq!(a.colors, b.colors);
        let mut child = a.clone();
        for (i, rule) in child.rules.iter_mut().enumerate() {
            if rng.chance(0.5) {
                *rule = b.rules[i];
            }
        }
        child
    }

    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for r in &self.rules {
            for byte in [r.write, r.turn.as_u8(), r.next] {
                h ^= byte as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    /// Compact wire/disk representation: three bytes per rule entry.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(self.rules.len() * 6);
        for r in &self.rules {
            s.push_str(&format!(
                "{:02x}{:02x}{:02x}",
                r.write,
                r.turn.as_u8(),
                r.next
            ));
        }
        s
    }

    pub fn from_hex(states: u8, colors: u8, hex: &str) -> Result<Self, String> {
        let hex = hex.trim();
        if hex.len() != states as usize * colors as usize * 6 {
            return Err(format!(
                "genome hex has {} chars, expected {}",
                hex.len(),
                states as usize * colors as usize * 6
            ));
        }
        let mut g = Genome::new(states, colors);
        for (i, r) in g.rules.iter_mut().enumerate() {
            let chunk = &hex[i * 6..i * 6 + 6];
            let write: u8 =
                u8::from_str_radix(&chunk[0..2], 16).map_err(|e| format!("bad write byte: {e}"))?;
            let turn: u8 =
                u8::from_str_radix(&chunk[2..4], 16).map_err(|e| format!("bad turn byte: {e}"))?;
            let next: u8 =
                u8::from_str_radix(&chunk[4..6], 16).map_err(|e| format!("bad state byte: {e}"))?;
            if write >= colors {
                return Err(format!("color {write} out of range"));
            }
            if next >= states {
                return Err(format!("state {next} out of range"));
            }
            r.write = write;
            r.turn = Turn::from_u8(turn).ok_or_else(|| format!("invalid turn {turn}"))?;
            r.next = next;
        }
        Ok(g)
    }

    /// Human readable table (state 0 rows first).
    pub fn table(&self) -> String {
        let mut out = String::new();
        out.push_str("state color -> write turn next\n");
        for s in 0..self.states {
            for c in 0..self.colors {
                let r = self.rule(s, c);
                out.push_str(&format!(
                    "  {:2}   {:2}   ->   {:2}   {:?}  {:2}\n",
                    s, c, r.write, r.turn, r.next
                ));
            }
        }
        out
    }
}

impl fmt::Display for Genome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Genome(states={}, colors={}, hex={})",
            self.states,
            self.colors,
            self.to_hex()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let mut rng = Rng::new(3);
        for (s, c) in [(2, 2), (2, 3), (3, 4)] {
            let g = Genome::random(s, c, &mut rng);
            let back = Genome::from_hex(s, c, &g.to_hex()).unwrap();
            assert_eq!(g, back);
        }
    }

    #[test]
    fn langton_table_is_canonical() {
        let g = Genome::langton();
        assert_eq!(
            g.rule(0, 0),
            Rule {
                write: 1,
                turn: Turn::Right,
                next: 0
            }
        );
        assert_eq!(
            g.rule(0, 1),
            Rule {
                write: 0,
                turn: Turn::Left,
                next: 0
            }
        );
    }

    #[test]
    fn invalid_hex_is_rejected() {
        assert!(Genome::from_hex(2, 2, "zz").is_err());
        assert!(Genome::from_hex(2, 2, "030000000000000000000000").is_err());
    }
}
