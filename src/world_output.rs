//! Monthly output estimates of the world's plants beyond Germany, and the gap between them and
//! the country's generation.
//!
//! A plant's output in a month is its capacity times its country's capacity factor for its source
//! and month: Ember's generation by source for the month (the year's average where Ember has no
//! months, see [`Figures::of`]) over the capacity Global Energy Monitor lists for that country and
//! source running in that month, capped at 1. Ember's sources are those of
//! [`crate::world_energy::MAPPING`]: wind not split by land and sea, coal not into lignite and
//! hard coal, so a plant's factor is that of its source's [`group`].
//!
//! GEM's list is incomplete (no rooftop solar and other small units, nor TransitionZero's rows),
//! so where its capacity cannot make Ember's generation even at full load, the rest is a gap per
//! country and source: generation in no plant of the list. It is told as such, never spread onto
//! plants. With the cap, plants and gap add up to Ember's generation for every source.

use crate::energy::{Source, SOURCES};
use crate::world_energy::{Basis, Figures};

/// The source whose factor a plant's output takes: Ember's category, drawn as `MAPPING` draws it.
pub fn group(s: Source) -> Source {
    match s {
        Source::WindOffshore => Source::WindOnshore,
        Source::Lignite => Source::HardCoal,
        s => s,
    }
}

/// The share of `capacity` (kW) that makes `generation` (average kW), from 0 to 1; 0 without
/// capacity.
pub fn factor(generation: f32, capacity: f32) -> f32 {
    if capacity > 0.0 && generation.is_finite() {
        (generation / capacity).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One country's estimate for one month, all by [`group`] sources (the others stay 0), in kW.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub basis: Basis,
    /// Ember's generation.
    pub generation: [f32; SOURCES],
    /// The capacity of the country's plants in GEM's list, running in the month.
    pub capacity: [f32; SOURCES],
    /// Their capacity factor.
    pub factor: [f32; SOURCES],
}

impl Estimate {
    /// From Ember's generation and GEM's capacity, each by source; both are taken by [`group`].
    pub fn new(basis: Basis, generation: [f32; SOURCES], capacity: [f32; SOURCES]) -> Estimate {
        let (mut g, mut c) = ([0.0; SOURCES], [0.0; SOURCES]);
        for s in Source::ALL {
            g[group(s) as usize] += generation[s as usize].max(0.0);
            c[group(s) as usize] += capacity[s as usize].max(0.0);
        }
        let factor = std::array::from_fn(|s| factor(g[s], c[s]));
        Estimate { basis, generation: g, capacity: c, factor }
    }

    /// From a country's figures for the month.
    pub fn of(f: &Figures, capacity: [f32; SOURCES]) -> Estimate {
        Estimate::new(f.basis, f.kw([true; SOURCES]), capacity)
    }

    /// The capacity factor a plant of `s` takes.
    pub fn factor_of(&self, s: Source) -> f32 {
        self.factor[group(s) as usize]
    }

    /// The plants' estimated output summed, by source.
    pub fn plants(&self) -> [f32; SOURCES] {
        std::array::from_fn(|s| self.capacity[s] * self.factor[s])
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

    #[test]
    fn the_factor_is_generation_over_capacity_capped_at_one() {
        assert_eq!(factor(250.0, 1000.0), 0.25);
        assert_eq!(factor(3000.0, 1000.0), 1.0);
        assert_eq!(factor(-5.0, 1000.0), 0.0);
        assert_eq!(factor(100.0, 0.0), 0.0);
        let e = Estimate::new(
            Basis::Month(2025, 6),
            by(&[(Source::Nuclear, 700.0), (Source::Solar, 900.0), (Source::WindOnshore, 300.0), (Source::Gas, 50.0)]),
            // Wind at sea shares Ember's wind; lignite and hard coal its coal.
            by(&[(Source::Nuclear, 1000.0), (Source::Solar, 600.0), (Source::WindOnshore, 500.0), (Source::WindOffshore, 500.0), (Source::Lignite, 200.0)]),
        );
        assert!((e.factor_of(Source::Nuclear) - 0.7).abs() < 1e-6);
        assert_eq!(e.factor_of(Source::Solar), 1.0, "capped");
        assert!((e.factor_of(Source::WindOffshore) - 0.3).abs() < 1e-6);
        assert_eq!(e.factor_of(Source::WindOnshore), e.factor_of(Source::WindOffshore));
        // Coal plants without coal generation make nothing; gas without plants is all gap.
        assert_eq!(e.factor_of(Source::Lignite), 0.0);
        let gap = e.gap();
        assert!(gap[Source::Nuclear as usize].abs() < 1e-3);
        assert!((gap[Source::Solar as usize] - 300.0).abs() < 1e-3);
        assert_eq!(gap[Source::Gas as usize], 50.0);
        assert!(gap.iter().all(|g| *g >= 0.0));
        // Plants and gap make Ember's figure for every source.
        let plants = e.plants();
        for s in 0..SOURCES {
            assert!((plants[s] + gap[s] - e.generation[s]).abs() < 1e-3, "{s}");
        }
        assert!((e.plant_share().unwrap() - (700.0 + 600.0 + 300.0) / 1950.0).abs() < 1e-5);
    }

    #[test]
    fn a_month_without_ember_months_takes_the_years_average() {
        let owid = Owid::bundled().unwrap();
        let c = owid.countries().iter().find(|c| !c.has_monthly() && c.year(Series::Nuclear, 2020).is_some_and(|v| v > 0.0)).or_else(|| owid.countries().iter().find(|c| !c.has_monthly() && c.year(Series::Hydro, 2020).is_some_and(|v| v > 0.0))).unwrap();
        let f = Figures::of(c, (2020, 3)).unwrap();
        assert_eq!(f.basis, Basis::Year(2020));
        let series = if c.year(Series::Nuclear, 2020).is_some_and(|v| v > 0.0) { (Series::Nuclear, Source::Nuclear) } else { (Series::Hydro, Source::Hydro) };
        let average = year_kw(c.year(series.0, 2020).unwrap(), 2020);
        let e = Estimate::of(&f, by(&[(series.1, average * 4.0)]));
        assert!((e.factor_of(series.1) - 0.25).abs() < 1e-4, "{}", e.factor_of(series.1));
        // With monthly figures, the month's own.
        let de = owid.country("FRA").unwrap();
        let f = Figures::of(de, (2025, 1)).unwrap();
        let nuclear = month_kw(de.month(Series::Nuclear, 2025, 1).unwrap(), 2025, 1);
        let e = Estimate::of(&f, by(&[(Source::Nuclear, nuclear * 2.0)]));
        assert!((e.factor_of(Source::Nuclear) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn months_count_from_1900() {
        assert_eq!(year_month(0), (1900, 1));
        assert_eq!(year_month(month_index((2025, 12))), (2025, 12));
        assert_eq!(month_index((2026, 9)), crate::energy::month_of(1_788_220_800.0));
    }
}
