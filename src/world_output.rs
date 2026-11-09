//! Monthly output estimates of the world's plants beyond Germany, and the gap between them and
//! the country's generation.
//!
//! A plant's output in a month is its capacity times its country's capacity factor for its kind
//! and month. The factor makes the plants of GEM's list (Global Energy Monitor) produce Ember's
//! generation of their source in that month (the year's average where Ember has no months, see
//! [`Figures::of`]), but never more than [`MAX_FACTOR`] allows for their kind: utility-scale
//! solar cannot run at 75% of its capacity over a month, so Ember's figure beyond what the listed
//! plants can make is not theirs. Ember's sources are those of [`crate::world_energy::MAPPING`]:
//! wind not split by land and sea, coal not into lignite and hard coal, hydro with pumped
//! storage, bioenergy with the other renewables; within such a [`group`], all plants run at one
//! factor up to their kind's ceiling, those whose ceiling is lower stopping there.
//!
//! GEM's list is incomplete (no rooftop solar and other small units, nor TransitionZero's rows),
//! so what is left over is a gap per country and source: generation in no plant of the list. It
//! is told as such, never spread onto plants. Plants and gap add up to Ember's generation for
//! every source.

use crate::energy::{Source, SOURCES};
use crate::world_energy::{Basis, Figures};

/// The kinds a plant's factor is capped by: the program's sources in the order of `Source::ALL`,
/// then pumped storage (drawn as hydro).
pub const KINDS: usize = SOURCES + 1;
/// The kind of pumped storage plants.
pub const PUMPED: usize = SOURCES;

/// The highest monthly capacity factor a plant of each kind is given, as an average over the
/// month. Physically plausible ceilings, not typical values:
///
/// | Kind | Ceiling | Why |
/// |---|---|---|
/// | Solar | 0.30 | Utility-scale solar, even tracking in deserts in summer, stays near or below this |
/// | Wind on land | 0.55 | Best sites in the windiest months |
/// | Wind at sea | 0.65 | Best North Sea months |
/// | Hydro | 0.90 | Run-of-river and storage plants in the wet season |
/// | Biomass (and geothermal) | 0.95 | Steady thermal plants: outages and upkeep aside |
/// | Lignite, hard coal, gas, oil, other fossil | 0.95 | The same |
/// | Nuclear | 0.95 | The same |
/// | Pumped storage | 0.30 | Generation of a few hours a day |
pub const MAX_FACTOR: [f32; KINDS] = [0.30, 0.55, 0.65, 0.90, 0.95, 0.95, 0.95, 0.95, 0.95, 0.95, 0.95, 0.30];

/// The source whose factor a plant's output takes: Ember's category, drawn as `MAPPING` draws it.
pub fn group(s: Source) -> Source {
    match s {
        Source::WindOffshore => Source::WindOnshore,
        Source::Lignite => Source::HardCoal,
        s => s,
    }
}

/// The source a kind belongs to.
pub fn source_of(kind: usize) -> Source {
    if kind == PUMPED {
        Source::Hydro
    } else {
        Source::ALL[kind]
    }
}

/// The factors of the kinds of one group: with `capacity` (kW) of each kind, the factors that make
/// `generation` (average kW), each at most its kind's [`MAX_FACTOR`]. All run at one factor,
/// those with a lower ceiling stopping at it; if even all at their ceilings cannot make the
/// generation, each runs at its ceiling. 0 without capacity.
pub fn factors(generation: f32, capacity: &[(usize, f32)]) -> Vec<f32> {
    let made = |f: f32| capacity.iter().map(|&(k, c)| c.max(0.0) * f.min(MAX_FACTOR[k])).sum::<f32>();
    let generation = if generation.is_finite() { generation.max(0.0) } else { 0.0 };
    let top = capacity.iter().map(|&(k, _)| MAX_FACTOR[k]).fold(0.0, f32::max);
    let f = if generation <= 0.0 {
        0.0
    } else if made(top) <= generation {
        top
    } else {
        let (mut lo, mut hi) = (0.0f32, top);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if made(mid) < generation {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    };
    capacity.iter().map(|&(k, c)| if c > 0.0 { f.min(MAX_FACTOR[k]) } else { 0.0 }).collect()
}

/// One country's estimate for one month, in kW.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub basis: Basis,
    /// Ember's generation, by [`group`] sources (the others stay 0).
    pub generation: [f32; SOURCES],
    /// The capacity of the country's plants in GEM's list running in the month, by kind.
    pub capacity: [f32; KINDS],
    /// Their capacity factor, by kind.
    pub factor: [f32; KINDS],
}

impl Estimate {
    /// From Ember's generation by source (taken by [`group`]) and GEM's capacity by kind.
    pub fn new(basis: Basis, generation: [f32; SOURCES], capacity: [f32; KINDS]) -> Estimate {
        let mut g = [0.0; SOURCES];
        for s in Source::ALL {
            g[group(s) as usize] += generation[s as usize].max(0.0);
        }
        let capacity = capacity.map(|c| c.max(0.0));
        let mut factor = [0.0; KINDS];
        for head in Source::ALL.into_iter().filter(|s| group(*s) == *s) {
            let kinds: Vec<(usize, f32)> = (0..KINDS).filter(|&k| group(source_of(k)) == head).map(|k| (k, capacity[k])).collect();
            for (&(k, _), f) in kinds.iter().zip(factors(g[head as usize], &kinds)) {
                factor[k] = f;
            }
        }
        Estimate { basis, generation: g, capacity, factor }
    }

    /// From a country's figures for the month.
    pub fn of(f: &Figures, capacity: [f32; KINDS]) -> Estimate {
        Estimate::new(f.basis, f.kw([true; SOURCES]), capacity)
    }

    /// The capacity factor a plant of `kind` takes.
    pub fn factor_of(&self, kind: usize) -> f32 {
        self.factor.get(kind).copied().unwrap_or(0.0)
    }

    /// The plants' estimated output summed, by [`group`] source.
    pub fn plants(&self) -> [f32; SOURCES] {
        let mut out = [0.0; SOURCES];
        for k in 0..KINDS {
            out[group(source_of(k)) as usize] += self.capacity[k] * self.factor[k];
        }
        out
    }

    /// Ember's generation that no plant of the list accounts for, by source; never below 0.
    pub fn gap(&self) -> [f32; SOURCES] {
        let plants = self.plants();
        std::array::from_fn(|s| (self.generation[s] - plants[s]).max(0.0))
    }

    /// The share of the generation the plants account for, 0 to 1; `None` without generation.
    pub fn plant_share(&self) -> Option<f32> {
        let total: f32 = self.generation.iter().sum();
        (total > 0.0).then(|| (self.plants().iter().sum::<f32>() / total).min(1.0))
    }
}

/// A month index (months since January 1900) as year and month.
pub fn year_month(month: u16) -> (i32, u32) {
    (1900 + month as i32 / 12, month as u32 % 12 + 1)
}

/// A year and month as a month index.
pub fn month_index((year, month): (i32, u32)) -> u16 {
    ((year - 1900) * 12 + month.clamp(1, 12) as i32 - 1).clamp(0, u16::MAX as i32 - 1) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owid::{Owid, Series};
    use crate::world_energy::{month_kw, year_kw};

    fn by(pairs: &[(Source, f32)]) -> [f32; SOURCES] {
        let mut out = [0.0; SOURCES];
        for &(s, v) in pairs {
            out[s as usize] += v;
        }
        out
    }

    fn kinds(pairs: &[(usize, f32)]) -> [f32; KINDS] {
        let mut out = [0.0; KINDS];
        for &(k, v) in pairs {
            out[k] += v;
        }
        out
    }

    const fn k(s: Source) -> usize {
        s as usize
    }

    #[test]
    fn the_factor_is_generation_over_capacity_up_to_the_kinds_ceiling() {
        assert_eq!(MAX_FACTOR.len(), KINDS);
        let one = |g: f32, k: usize, c: f32| factors(g, &[(k, c)])[0];
        assert!((one(250.0, k(Source::Nuclear), 1000.0) - 0.25).abs() < 1e-5);
        assert_eq!(one(3000.0, k(Source::Nuclear), 1000.0), 0.95);
        assert_eq!(one(900.0, k(Source::Solar), 1000.0), 0.30);
        assert_eq!(one(-5.0, k(Source::Gas), 1000.0), 0.0);
        assert_eq!(one(100.0, k(Source::Gas), 0.0), 0.0);
        let e = Estimate::new(
            Basis::Month(2025, 6),
            by(&[(Source::Nuclear, 700.0), (Source::Solar, 900.0), (Source::WindOnshore, 300.0), (Source::Gas, 50.0), (Source::Hydro, 1000.0)]),
            // Wind at sea shares Ember's wind; lignite and hard coal its coal; pumped storage its hydro.
            kinds(&[(k(Source::Nuclear), 1000.0), (k(Source::Solar), 600.0), (k(Source::WindOnshore), 500.0), (k(Source::WindOffshore), 500.0), (k(Source::Lignite), 200.0), (k(Source::Hydro), 1000.0), (PUMPED, 1000.0)]),
        );
        assert!((e.factor_of(k(Source::Nuclear)) - 0.7).abs() < 1e-5);
        assert_eq!(e.factor_of(k(Source::Solar)), 0.30, "capped");
        assert!((e.factor_of(k(Source::WindOffshore)) - 0.3).abs() < 1e-5);
        assert_eq!(e.factor_of(k(Source::WindOnshore)), e.factor_of(k(Source::WindOffshore)));
        // Hydro: pumped storage stops at its ceiling, the rest takes up the generation.
        assert_eq!(e.factor_of(PUMPED), 0.30);
        assert!((e.factor_of(k(Source::Hydro)) - 0.70).abs() < 1e-4, "{}", e.factor_of(k(Source::Hydro)));
        // Coal plants without coal generation make nothing; gas without plants is all gap.
        assert_eq!(e.factor_of(k(Source::Lignite)), 0.0);
        let gap = e.gap();
        assert!(gap[Source::Nuclear as usize].abs() < 1e-2);
        // Where the ceiling binds, the rest is the gap: 900 against 600 × 0.3.
        assert!((gap[Source::Solar as usize] - 720.0).abs() < 1e-2);
        assert_eq!(gap[Source::Gas as usize], 50.0);
        assert!(gap.iter().all(|g| *g >= 0.0));
        // Plants and gap make Ember's figure for every source.
        let plants = e.plants();
        for s in 0..SOURCES {
            assert!((plants[s] + gap[s] - e.generation[s]).abs() < 1e-2, "{s}");
        }
        assert!((e.plant_share().unwrap() - (700.0 + 180.0 + 300.0 + 1000.0) / 2950.0).abs() < 1e-4);
    }

    #[test]
    fn a_month_without_ember_months_takes_the_years_average() {
        let owid = Owid::bundled().unwrap();
        let c = owid.countries().iter().find(|c| !c.has_monthly() && c.year(Series::Nuclear, 2020).is_some_and(|v| v > 0.0)).or_else(|| owid.countries().iter().find(|c| !c.has_monthly() && c.year(Series::Hydro, 2020).is_some_and(|v| v > 0.0))).unwrap();
        let f = Figures::of(c, (2020, 3)).unwrap();
        assert_eq!(f.basis, Basis::Year(2020));
        let series = if c.year(Series::Nuclear, 2020).is_some_and(|v| v > 0.0) { (Series::Nuclear, Source::Nuclear) } else { (Series::Hydro, Source::Hydro) };
        let average = year_kw(c.year(series.0, 2020).unwrap(), 2020);
        let e = Estimate::of(&f, kinds(&[(k(series.1), average * 4.0)]));
        assert!((e.factor_of(k(series.1)) - 0.25).abs() < 1e-4, "{}", e.factor_of(k(series.1)));
        // With monthly figures, the month's own.
        let fr = owid.country("FRA").unwrap();
        let f = Figures::of(fr, (2025, 1)).unwrap();
        let nuclear = month_kw(fr.month(Series::Nuclear, 2025, 1).unwrap(), 2025, 1);
        let e = Estimate::of(&f, kinds(&[(k(Source::Nuclear), nuclear * 2.0)]));
        assert!((e.factor_of(k(Source::Nuclear)) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn months_count_from_1900() {
        assert_eq!(year_month(0), (1900, 1));
        assert_eq!(year_month(month_index((2025, 12))), (2025, 12));
        assert_eq!(month_index((2026, 9)), crate::energy::month_of(1_788_220_800.0));
    }
}
