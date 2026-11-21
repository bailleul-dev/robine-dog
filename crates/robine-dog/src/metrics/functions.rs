//! Datadog's metric functions, on one series sampled on a regular grid
//! (`None`: no point in that interval).
//!
//! Smoothing (`ewma_N`, `median_N`, `moving_rollup`), interpolation (`fill`),
//! arithmetic over time (`integral`, `monotonic_diff`), regressions
//! (`trend_line`, `robust_trend`, `piecewise_constant`) and the algorithms:
//! `anomalies` (a band of expected values), `forecast` (future values and a
//! band) and `outliers` (which series of a group stray from the others).

pub type Values = Vec<Option<f64>>;

/// The expected range of a series, point by point.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Band {
    pub lower: Values,
    pub upper: Values,
}

fn known(v: &[Option<f64>]) -> Vec<(usize, f64)> {
    v.iter()
        .enumerate()
        .filter_map(|(i, x)| x.map(|x| (i, x)))
        .collect()
}

fn median_of(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// Median absolute deviation, scaled to estimate a standard deviation.
fn mad_sigma(v: &[f64]) -> Option<f64> {
    let m = median_of(&mut v.to_vec())?;
    let mut dev: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    median_of(&mut dev).map(|d| d * 1.4826)
}

fn mean_std(v: &[f64]) -> Option<(f64, f64)> {
    if v.is_empty() {
        return None;
    }
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    Some((mean, var.sqrt()))
}

/// A spread never quite zero, so a flat series still gets a visible band.
fn floor_sigma(sigma: f64, level: f64) -> f64 {
    sigma.max(level.abs() * 0.01).max(1e-9)
}

/// `ewma_N`: exponentially weighted moving average, `alpha = 2 / (N + 1)`.
/// Gaps stay gaps without resetting the average.
pub fn ewma(v: &[Option<f64>], span: usize) -> Values {
    let alpha = 2.0 / (span.max(1) as f64 + 1.0);
    let mut acc: Option<f64> = None;
    v.iter()
        .map(|x| {
            let x = (*x)?;
            let next = match acc {
                Some(a) => a + alpha * (x - a),
                None => x,
            };
            acc = Some(next);
            Some(next)
        })
        .collect()
}

/// Aggregates the known values of the last `w` points (this one included).
pub fn rolling(v: &[Option<f64>], w: usize, how: &str) -> Values {
    let w = w.max(1);
    (0..v.len())
        .map(|i| {
            v[i]?;
            let mut win: Vec<f64> = v[i + 1 - w.min(i + 1)..=i]
                .iter()
                .flatten()
                .copied()
                .collect();
            match how {
                "median" => median_of(&mut win),
                "sum" => Some(win.iter().sum()),
                "min" => win.iter().copied().reduce(f64::min),
                "max" => win.iter().copied().reduce(f64::max),
                "count" => Some(win.len() as f64),
                _ => mean_std(&win).map(|(m, _)| m),
            }
        })
        .collect()
}

/// `median_N`: rolling median of the last `N` points.
pub fn median(v: &[Option<f64>], w: usize) -> Values {
    rolling(v, w, "median")
}

/// `fill(null|zero|linear|last, limit)`: fills gaps of at most `limit`
/// points (all of them without a limit). Leading gaps are never filled.
pub fn fill(v: &[Option<f64>], mode: &str, limit: Option<usize>) -> Values {
    let mut out = v.to_vec();
    if mode == "null" {
        return out;
    }
    let pts = known(v);
    let ok = |gap: usize| limit.is_none_or(|l| gap <= l);
    for w in pts.windows(2) {
        let ((a, va), (b, vb)) = (w[0], w[1]);
        let gap = b - a - 1;
        if gap == 0 || !ok(gap) {
            continue;
        }
        for (k, slot) in out.iter_mut().enumerate().take(b).skip(a + 1) {
            *slot = Some(match mode {
                "zero" => 0.0,
                "linear" => va + (vb - va) * (k - a) as f64 / (b - a) as f64,
                _ => va,
            });
        }
    }
    // After the last point: only `zero` and `last` extend it.
    if let Some(&(last, vl)) = pts.last() {
        let gap = v.len() - last - 1;
        if gap > 0 && ok(gap) && matches!(mode, "zero" | "last") {
            for slot in out.iter_mut().skip(last + 1) {
                *slot = Some(if mode == "zero" { 0.0 } else { vl });
            }
        }
    }
    out
}

/// `integral`: the running sum of value × interval (seconds).
pub fn integral(v: &[Option<f64>], interval_s: f64) -> Values {
    let mut acc = 0.0;
    v.iter()
        .map(|x| {
            x.map(|x| {
                acc += x * interval_s;
                acc
            })
        })
        .collect()
}

/// `monotonic_diff`: like `diff`, but a drop (a counter reset) counts the
/// new value as the increase instead of going negative.
pub fn monotonic_diff(v: &[Option<f64>]) -> Values {
    let mut prev: Option<f64> = None;
    v.iter()
        .map(|x| {
            let d = match (*x, prev) {
                (Some(x), Some(p)) if x >= p => Some(x - p),
                (Some(x), Some(_)) => Some(x),
                _ => None,
            };
            if x.is_some() {
                prev = *x;
            }
            d
        })
        .collect()
}

/// `count_nonzero` / `count_not_null`: how many series, at each point.
pub fn count_series(series: &[Values], len: usize, nonzero: bool) -> Values {
    (0..len)
        .map(|i| {
            Some(
                series
                    .iter()
                    .filter(|s| match s.get(i).copied().flatten() {
                        Some(v) => !nonzero || v != 0.0,
                        None => false,
                    })
                    .count() as f64,
            )
        })
        .collect()
}

/// Least squares over the known points: `(slope, intercept)` in points.
fn ols(pts: &[(usize, f64)]) -> Option<(f64, f64)> {
    if pts.len() < 2 {
        return pts.first().map(|&(_, y)| (0.0, y));
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0 as f64).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pts.iter().map(|p| (p.0 as f64 - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 as f64 - mx) * (p.1 - my)).sum();
    let slope = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    Some((slope, my - slope * mx))
}

fn line(len: usize, (slope, b): (f64, f64)) -> Values {
    (0..len).map(|i| Some(b + slope * i as f64)).collect()
}

/// `trend_line`: the ordinary least squares line.
pub fn trend_line(v: &[Option<f64>]) -> Values {
    ols(&known(v)).map_or_else(|| vec![None; v.len()], |f| line(v.len(), f))
}

/// `robust_trend`: a Theil–Sen line (median of the pairwise slopes), which
/// a few outliers don't pull. Long series are subsampled to 300 points.
pub fn robust_trend(v: &[Option<f64>]) -> Values {
    let pts = known(v);
    if pts.len() < 2 {
        return trend_line(v);
    }
    let step = pts.len().div_ceil(300);
    let pts: Vec<(usize, f64)> = pts.into_iter().step_by(step).collect();
    let mut slopes = Vec::with_capacity(pts.len() * pts.len() / 2);
    for (i, a) in pts.iter().enumerate() {
        for b in &pts[i + 1..] {
            slopes.push((b.1 - a.1) / (b.0 - a.0) as f64);
        }
    }
    let slope = median_of(&mut slopes).unwrap_or(0.0);
    let mut icpt: Vec<f64> = pts.iter().map(|p| p.1 - slope * p.0 as f64).collect();
    let b = median_of(&mut icpt).unwrap_or(0.0);
    line(v.len(), (slope, b))
}

/// `piecewise_constant`: the series as steps, found by binary segmentation.
/// A split is kept when it lowers the squared error by more than a penalty
/// scaled on the noise (from the differences between neighbours).
pub fn piecewise_constant(v: &[Option<f64>]) -> Values {
    let pts = known(v);
    if pts.len() < 2 {
        return v.to_vec();
    }
    let diffs: Vec<f64> = pts.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let noise = mad_sigma(&diffs).unwrap_or(0.0) / std::f64::consts::SQRT_2;
    let level = pts.iter().map(|p| p.1.abs()).fold(0.0, f64::max);
    let noise = floor_sigma(noise, level * 0.1);
    let penalty = 3.0 * noise * noise * (pts.len() as f64).ln().max(1.0);
    let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let mut cuts = vec![0, ys.len()];
    let sse = |s: &[f64]| {
        let m = s.iter().sum::<f64>() / s.len() as f64;
        s.iter().map(|x| (x - m).powi(2)).sum::<f64>()
    };
    let mut stack = vec![(0, ys.len())];
    while let Some((a, b)) = stack.pop() {
        if b - a < 2 || cuts.len() > 64 {
            continue;
        }
        let whole = sse(&ys[a..b]);
        let best = (a + 1..b)
            .map(|k| (k, sse(&ys[a..k]) + sse(&ys[k..b])))
            .min_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, e)) = best
            && whole - e > penalty
        {
            cuts.push(k);
            stack.push((a, k));
            stack.push((k, b));
        }
    }
    cuts.sort_unstable();
    let mut out = vec![None; v.len()];
    for w in cuts.windows(2) {
        let seg = &ys[w[0]..w[1]];
        let m = seg.iter().sum::<f64>() / seg.len() as f64;
        let (start, end) = (pts[w[0]].0, pts[w[1] - 1].0);
        // Steps span gaps up to the next segment's first point.
        let stop = pts.get(w[1]).map_or(end + 1, |p| p.0);
        for slot in out.iter_mut().take(stop).skip(start) {
            *slot = Some(m);
        }
    }
    out
}

/// The `anomalies()` algorithms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnomalyAlgo {
    /// A lagging rolling mean and deviation: no seasonality, adapts fast.
    Basic,
    /// The previous season, shifted to the current level: follows level
    /// shifts quickly.
    Agile,
    /// The median of the previous seasons: stable, slow to follow shifts.
    Robust,
}

impl AnomalyAlgo {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "basic" => Self::Basic,
            "agile" => Self::Agile,
            "robust" | "adaptive" => Self::Robust,
            _ => return None,
        })
    }
}

/// The band of expected values of `cur`. `history` holds older windows on
/// the same grid, oldest first: for `basic` the window just before (its
/// lagging values), for `agile`/`robust` the same window 1, 2, 3 seasons
/// ago. Without history, `agile`/`robust` fall back to `basic`.
pub fn anomaly_band(
    cur: &[Option<f64>],
    history: &[Values],
    algo: AnomalyAlgo,
    bounds: f64,
) -> Band {
    let n = cur.len();
    let bounds = if bounds > 0.0 { bounds } else { 2.0 };
    let seasons: Vec<&Values> = history.iter().filter(|h| h.len() == n).collect();
    if algo == AnomalyAlgo::Basic || seasons.iter().all(|h| h.iter().all(Option::is_none)) {
        return basic_band(cur, history.last().map_or(&[][..], |h| &h[..]), bounds);
    }
    // Seasonal: the most recent season is the last of `history`.
    let last = seasons.last().unwrap();
    let expected: Values = (0..n)
        .map(|i| {
            let mut at: Vec<f64> = seasons.iter().filter_map(|h| h[i]).collect();
            match algo {
                AnomalyAlgo::Robust => median_of(&mut at),
                _ => last[i].or_else(|| median_of(&mut at)),
            }
        })
        .collect();
    // How far seasons stray from one another: the noise to allow.
    let mut residuals: Vec<f64> = Vec::new();
    for w in seasons.windows(2) {
        for (a, b) in w[0].iter().zip(w[1].iter()) {
            if let (Some(a), Some(b)) = (a, b) {
                residuals.push(b - a);
            }
        }
    }
    if residuals.len() < 3 {
        // One season only: its own variation around its rolling mean.
        let r = rolling(last, (n / 10).max(3), "avg");
        residuals = (0..n).filter_map(|i| Some(last[i]? - r[i]?)).collect();
    }
    let level = expected.iter().flatten().map(|x| x.abs()).sum::<f64>()
        / expected.iter().flatten().count().max(1) as f64;
    let sigma = floor_sigma(mad_sigma(&residuals).unwrap_or(0.0), level);
    // Agile follows level shifts: the recent median gap between the series
    // and the season it repeats.
    let w = (n / 8).clamp(3, 60);
    let mut lower = vec![None; n];
    let mut upper = vec![None; n];
    for i in 0..n {
        let Some(e) = expected[i] else { continue };
        let shift = if algo == AnomalyAlgo::Agile {
            let mut gaps: Vec<f64> = (i.saturating_sub(w)..i)
                .filter_map(|j| Some(cur[j]? - expected[j]?))
                .collect();
            median_of(&mut gaps).unwrap_or(0.0)
        } else {
            0.0
        };
        lower[i] = Some(e + shift - bounds * sigma);
        upper[i] = Some(e + shift + bounds * sigma);
    }
    // Where no earlier season has a point (history younger than three
    // seasons), the series is judged on its own recent past.
    if lower.iter().any(Option::is_none) {
        let basic = basic_band(cur, &[], bounds);
        for i in 0..n {
            if lower[i].is_none() {
                lower[i] = basic.lower[i];
                upper[i] = basic.upper[i];
            }
        }
    }
    Band { lower, upper }
}

/// Each point judged on the `w` points before it (`prev`: the window just
/// before `cur`, possibly empty): their mean ± `bounds` deviations.
fn basic_band(cur: &[Option<f64>], prev: &[Option<f64>], bounds: f64) -> Band {
    let n = cur.len();
    let all: Values = prev.iter().chain(cur).copied().collect();
    let w = (n / 4).clamp(5, 120);
    let mut lower = vec![None; n];
    let mut upper = vec![None; n];
    for i in 0..n {
        let p = prev.len() + i;
        let win: Vec<f64> = all[p.saturating_sub(w)..p]
            .iter()
            .flatten()
            .copied()
            .collect();
        if win.len() < 3 {
            continue;
        }
        let (m, s) = mean_std(&win).unwrap();
        let s = floor_sigma(s, m);
        lower[i] = Some(m - bounds * s);
        upper[i] = Some(m + bounds * s);
    }
    Band { lower, upper }
}

/// Whether a point leaves its band (`direction`: both, above, below).
pub fn is_anomalous(v: f64, lower: f64, upper: f64, direction: &str) -> bool {
    match direction {
        "above" => v > upper,
        "below" => v < lower,
        _ => v > upper || v < lower,
    }
}

/// The share of the known points of `cur` outside the band (0 to 1), as an
/// anomaly monitor evaluates its window.
pub fn anomalous_share(cur: &[Option<f64>], band: &Band, direction: &str) -> Option<f64> {
    let judged: Vec<bool> = cur
        .iter()
        .zip(band.lower.iter().zip(&band.upper))
        .filter_map(|(v, (l, u))| Some(is_anomalous((*v)?, (*l)?, (*u)?, direction)))
        .collect();
    (!judged.is_empty()).then(|| judged.iter().filter(|x| **x).count() as f64 / judged.len() as f64)
}

/// `forecast(q, 'linear'|'seasonal', deviations)`: predicts the points after
/// the last known one. Returns the series with those points filled in, and a
/// band on the predicted part only. `season` is in points (`seasonal`
/// falls back to `linear` when fewer than two seasons are known); `history`
/// is the window before, to learn from.
pub fn forecast(
    cur: &[Option<f64>],
    history: &[Option<f64>],
    seasonal: bool,
    deviations: f64,
    season: usize,
) -> (Values, Band) {
    let n = cur.len();
    let all: Values = history.iter().chain(cur).copied().collect();
    let off = history.len();
    let pts = known(&all);
    let mut out = cur.to_vec();
    let mut band = Band {
        lower: vec![None; n],
        upper: vec![None; n],
    };
    let Some(&(last, _)) = pts.last() else {
        return (out, band);
    };
    let dev = if deviations > 0.0 { deviations } else { 1.0 };
    let filled = fill(&all, "linear", None);
    let use_season = seasonal && season >= 2 && pts.len() >= 2 * season && last + 1 >= 2 * season;
    if use_season {
        // Seasonal naive with drift: the value one season ago plus the
        // average change from one season to the next.
        let changes: Vec<f64> = (season..=last)
            .filter_map(|i| Some(filled[i]? - filled[i - season]?))
            .collect();
        let drift = mean_std(&changes).map_or(0.0, |(m, _)| m);
        let sigma = floor_sigma(
            mad_sigma(&changes.iter().map(|c| c - drift).collect::<Vec<_>>()).unwrap_or(0.0),
            pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
        );
        let mut ext = filled.clone();
        for i in last + 1..all.len() {
            let base = ext[i - season];
            ext[i] = base.map(|b| b + drift);
            if i >= off {
                let steps = ((i - last) as f64 / season as f64).ceil();
                out[i - off] = ext[i];
                band.lower[i - off] = ext[i].map(|v| v - dev * sigma * steps.sqrt());
                band.upper[i - off] = ext[i].map(|v| v + dev * sigma * steps.sqrt());
            }
        }
        return (out, band);
    }
    // Linear: least squares on the recent points (the last half at most
    // 500), the band widening with the distance.
    let recent: Vec<(usize, f64)> =
        pts[pts.len().saturating_sub(500.max(pts.len() / 2))..].to_vec();
    let (slope, b) = ols(&recent).unwrap_or((0.0, 0.0));
    let res: Vec<f64> = recent
        .iter()
        .map(|&(i, y)| y - (b + slope * i as f64))
        .collect();
    let level = recent.iter().map(|p| p.1).sum::<f64>() / recent.len() as f64;
    let sigma = floor_sigma(mean_std(&res).map_or(0.0, |(_, s)| s), level);
    let span = recent.len().max(1) as f64;
    for i in last + 1..all.len() {
        if i < off {
            continue;
        }
        let p = b + slope * i as f64;
        let widen = (1.0 + (i - last) as f64 / span).sqrt();
        out[i - off] = Some(p);
        band.lower[i - off] = Some(p - dev * sigma * widen);
        band.upper[i - off] = Some(p + dev * sigma * widen);
    }
    (out, band)
}

/// The `outliers()` algorithms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutlierAlgo {
    Dbscan,
    ScaledDbscan,
    Mad,
    ScaledMad,
}

impl OutlierAlgo {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "dbscan" => Self::Dbscan,
            "scaleddbscan" => Self::ScaledDbscan,
            "mad" => Self::Mad,
            "scaledmad" => Self::ScaledMad,
            _ => return None,
        })
    }

    /// Datadog's default tolerance.
    pub fn default_tolerance(self) -> f64 {
        match self {
            Self::Dbscan | Self::ScaledDbscan => 3.0,
            Self::Mad | Self::ScaledMad => 3.0,
        }
    }
}

/// Which series of a group are outliers. MAD judges each point against the
/// median of the group at that time, and a series is an outlier when more
/// than `pct` percent of its points are; DBSCAN clusters the series by
/// distance and everything outside the main cluster is an outlier. The
/// scaled variants divide distances by the group's median level.
pub fn outliers(series: &[Values], algo: OutlierAlgo, tolerance: f64, pct: f64) -> Vec<bool> {
    let k = series.len();
    if k < 3 {
        return vec![false; k];
    }
    let len = series.iter().map(Vec::len).max().unwrap_or(0);
    let tol = if tolerance > 0.0 {
        tolerance
    } else {
        algo.default_tolerance()
    };
    let scaled = matches!(algo, OutlierAlgo::ScaledMad | OutlierAlgo::ScaledDbscan);
    let medians: Vec<Option<f64>> = (0..len)
        .map(|i| {
            median_of(
                &mut series
                    .iter()
                    .filter_map(|s| s.get(i).copied().flatten())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let scale = |i: usize| -> f64 {
        if scaled {
            medians[i].map_or(1.0, |m| m.abs().max(1e-9))
        } else {
            1.0
        }
    };
    match algo {
        OutlierAlgo::Mad | OutlierAlgo::ScaledMad => {
            let pct = if pct > 0.0 { pct } else { 20.0 };
            let sigmas: Vec<Option<f64>> = (0..len)
                .map(|i| {
                    let at: Vec<f64> = series
                        .iter()
                        .filter_map(|s| s.get(i).copied().flatten())
                        .map(|v| v / scale(i))
                        .collect();
                    mad_sigma(&at)
                })
                .collect();
            series
                .iter()
                .map(|s| {
                    let mut judged = 0;
                    let mut out = 0;
                    for i in 0..len {
                        let (Some(v), Some(m), Some(sd)) =
                            (s.get(i).copied().flatten(), medians[i], sigmas[i])
                        else {
                            continue;
                        };
                        judged += 1;
                        let d = ((v - m) / scale(i)).abs();
                        if d > tol * sd.max(1e-9) && d > 1e-9 {
                            out += 1;
                        }
                    }
                    judged > 0 && out as f64 * 100.0 / judged as f64 > pct
                })
                .collect()
        }
        OutlierAlgo::Dbscan | OutlierAlgo::ScaledDbscan => {
            // Distance between series: mean absolute gap over common points.
            let dist = |a: &Values, b: &Values| -> Option<f64> {
                let d: Vec<f64> = (0..len)
                    .filter_map(|i| {
                        Some(
                            ((a.get(i).copied().flatten()? - b.get(i).copied().flatten()?)
                                / scale(i))
                            .abs(),
                        )
                    })
                    .collect();
                mean_std(&d).map(|(m, _)| m)
            };
            let med: Values = medians.clone();
            // Each series' distance to the median series sets the scale.
            let mut to_median: Vec<f64> = series.iter().filter_map(|s| dist(s, &med)).collect();
            let eps = tol * median_of(&mut to_median).unwrap_or(0.0).max(1e-9);
            // Connected components of the "within eps" graph; the largest
            // one is normal.
            let mut comp = vec![usize::MAX; k];
            let mut next = 0;
            for start in 0..k {
                if comp[start] != usize::MAX {
                    continue;
                }
                comp[start] = next;
                let mut stack = vec![start];
                while let Some(a) = stack.pop() {
                    for b in 0..k {
                        if comp[b] == usize::MAX
                            && dist(&series[a], &series[b]).is_some_and(|d| d <= eps)
                        {
                            comp[b] = next;
                            stack.push(b);
                        }
                    }
                }
                next += 1;
            }
            let mut sizes = vec![0; next];
            for c in &comp {
                sizes[*c] += 1;
            }
            let main = (0..next).max_by_key(|c| sizes[*c]).unwrap_or(0);
            comp.iter().map(|c| *c != main).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[f64]) -> Values {
        v.iter().map(|x| Some(*x)).collect()
    }

    #[test]
    fn smoothing() {
        let e = ewma(&s(&[10.0, 20.0, 20.0]), 3);
        assert_eq!(e[0], Some(10.0));
        assert_eq!(e[1], Some(15.0));
        assert_eq!(e[2], Some(17.5));
        assert_eq!(
            median(&[Some(1.0), Some(100.0), Some(3.0), None], 3),
            vec![Some(1.0), Some(50.5), Some(3.0), None]
        );
        assert_eq!(
            rolling(&s(&[1.0, 2.0, 3.0]), 2, "sum"),
            vec![Some(1.0), Some(3.0), Some(5.0)]
        );
    }

    #[test]
    fn fills() {
        let v = vec![Some(1.0), None, None, Some(4.0), None];
        assert_eq!(
            fill(&v, "linear", None),
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), None]
        );
        assert_eq!(
            fill(&v, "zero", None),
            vec![Some(1.0), Some(0.0), Some(0.0), Some(4.0), Some(0.0)]
        );
        assert_eq!(fill(&v, "last", Some(1))[1], None);
        assert_eq!(fill(&v, "last", Some(1))[4], Some(4.0));
        assert_eq!(fill(&v, "null", None), v);
    }

    #[test]
    fn counters_and_integral() {
        assert_eq!(
            monotonic_diff(&s(&[5.0, 7.0, 2.0, 4.0])),
            vec![None, Some(2.0), Some(2.0), Some(2.0)]
        );
        assert_eq!(
            integral(&s(&[1.0, 2.0]), 10.0),
            vec![Some(10.0), Some(30.0)]
        );
        let c = count_series(&[s(&[0.0, 1.0]), vec![None, Some(2.0)]], 2, true);
        assert_eq!(c, vec![Some(0.0), Some(2.0)]);
        let c = count_series(&[s(&[0.0, 1.0]), vec![None, Some(2.0)]], 2, false);
        assert_eq!(c, vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn regressions() {
        let t = trend_line(&s(&[1.0, 2.0, 3.0, 4.0]));
        assert!((t[3].unwrap() - 4.0).abs() < 1e-9);
        // One huge outlier does not bend the robust trend.
        let r = robust_trend(&s(&[1.0, 2.0, 3.0, 1000.0, 5.0, 6.0]));
        assert!((r[5].unwrap() - 6.0).abs() < 1e-6);
        let p = piecewise_constant(&s(&[1.0, 1.1, 0.9, 1.0, 10.0, 10.1, 9.9, 10.0]));
        assert!((p[0].unwrap() - 1.0).abs() < 0.1);
        assert!((p[7].unwrap() - 10.0).abs() < 0.1);
    }

    #[test]
    fn basic_anomalies() {
        let calm: Values = (0..40).map(|i| Some(10.0 + (i % 3) as f64)).collect();
        let mut cur = calm.clone();
        cur[30] = Some(100.0);
        let band = anomaly_band(&cur, std::slice::from_ref(&calm), AnomalyAlgo::Basic, 2.0);
        assert!(!is_anomalous(
            11.0,
            band.lower[10].unwrap(),
            band.upper[10].unwrap(),
            "both"
        ));
        assert!(is_anomalous(
            100.0,
            band.lower[30].unwrap(),
            band.upper[30].unwrap(),
            "both"
        ));
        let share = anomalous_share(&cur, &band, "both").unwrap();
        assert!(share > 0.0 && share < 0.1);
    }

    #[test]
    fn seasonal_anomalies() {
        // The same daily shape every season; today a spike at noon.
        let day: Values = (0..24)
            .map(|h| Some(if (8..18).contains(&h) { 100.0 } else { 10.0 }))
            .collect();
        let mut today = day.clone();
        today[12] = Some(300.0);
        let hist = vec![day.clone(), day.clone(), day];
        for algo in [AnomalyAlgo::Agile, AnomalyAlgo::Robust] {
            let b = anomaly_band(&today, &hist, algo, 2.0);
            // A busy afternoon is expected, the spike is not.
            assert!(!is_anomalous(
                100.0,
                b.lower[14].unwrap(),
                b.upper[14].unwrap(),
                "both"
            ));
            assert!(is_anomalous(
                300.0,
                b.lower[12].unwrap(),
                b.upper[12].unwrap(),
                "above"
            ));
            assert!(!is_anomalous(
                300.0,
                b.lower[12].unwrap(),
                b.upper[12].unwrap(),
                "below"
            ));
        }
    }

    #[test]
    fn forecasts() {
        let mut cur: Values = (0..10).map(|i| Some(i as f64)).collect();
        cur.extend([None; 5]);
        let (f, band) = forecast(&cur, &[], false, 1.0, 0);
        assert!((f[14].unwrap() - 14.0).abs() < 1e-6);
        assert!(band.lower[14].unwrap() <= 14.0 && band.upper[14].unwrap() >= 14.0);
        assert_eq!(band.lower[3], None);
        // A repeating 4-point season.
        let mut cur: Values = (0..12)
            .map(|i| Some([0.0, 5.0, 10.0, 5.0][i % 4]))
            .collect();
        cur.extend([None; 4]);
        let (f, _) = forecast(&cur, &[], true, 1.0, 4);
        assert_eq!(
            f[12..]
                .iter()
                .map(|v| v.unwrap().round())
                .collect::<Vec<_>>(),
            vec![0.0, 5.0, 10.0, 5.0]
        );
    }

    #[test]
    fn finds_outliers() {
        let normal: Vec<Values> = (0..5).map(|k| s(&[10.0 + k as f64 * 0.1; 20])).collect();
        let mut group = normal.clone();
        group.push(s(&[50.0; 20]));
        for algo in [
            OutlierAlgo::Mad,
            OutlierAlgo::ScaledMad,
            OutlierAlgo::Dbscan,
            OutlierAlgo::ScaledDbscan,
        ] {
            let o = outliers(&group, algo, 3.0, 0.0);
            assert_eq!(o, vec![false, false, false, false, false, true], "{algo:?}");
        }
        assert_eq!(
            outliers(&normal[..2], OutlierAlgo::Mad, 3.0, 0.0),
            vec![false, false]
        );
    }
}
