//! Forecast "full in X days" from the own history of a volume.
//!
//! A least-squares line runs through the daily values of the used space of
//! the last [`FORECAST_DAYS`] days. The days until full are counted from the
//! point of the line at today, not from the last measured value, so a large
//! cleanup does not make the forecast jump: it follows the line.

use crate::history::Point;

const DAY: i64 = 86_400;

/// Days of history the line runs through, today included.
pub const FORECAST_DAYS: usize = 30;

/// Daily values needed in those days; gaps do not count.
pub const MIN_FORECAST_DAYS: usize = 7;

/// A volume that would only be full after more days than this has no trend.
pub const MAX_FORECAST_DAYS: f64 = 3650.0;

/// Result of the forecast for one volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    /// Days from `now` until the volume is full; 0 when the line is already
    /// at or above the size of the volume.
    pub days_to_full: f64,
    /// Time the volume is full, Unix seconds UTC.
    pub full_at: i64,
    /// Growth of the used space in bytes per day.
    pub growth_per_day: f64,
}

/// Start of the first day of the forecast window for `now`.
pub(crate) fn window_start(now: i64) -> i64 {
    now - now.rem_euclid(DAY) - (FORECAST_DAYS as i64 - 1) * DAY
}

/// Forecast from daily values of the used space (`used`, oldest first) and
/// the size of the volume `total` in bytes. `None` means "no trend": fewer
/// than [`MIN_FORECAST_DAYS`] daily values in the window, a flat or falling
/// line, a volume that would be full only after [`MAX_FORECAST_DAYS`], or an
/// unusable size.
pub fn forecast(used: &[Point], total: f64, now: i64) -> Option<Forecast> {
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    let today = now - now.rem_euclid(DAY);
    let from = window_start(now);
    let values: Vec<(f64, f64)> = used
        .iter()
        .filter(|p| p.at >= from && p.at <= now && p.mean.is_finite())
        .map(|p| ((p.at - today) as f64 / DAY as f64, p.mean))
        .collect();
    if values.len() < MIN_FORECAST_DAYS {
        return None;
    }

    let n = values.len() as f64;
    let mean_x = values.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = values.iter().map(|(_, y)| y).sum::<f64>() / n;
    let sxx: f64 = values.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
    let sxy: f64 = values
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    if !slope.is_finite() || slope <= 0.0 {
        return None;
    }

    let level_today = mean_y - slope * mean_x;
    let days = ((total - level_today) / slope).max(0.0);
    if !days.is_finite() || days > MAX_FORECAST_DAYS {
        return None;
    }
    Some(Forecast {
        days_to_full: days,
        full_at: now + (days * DAY as f64).round() as i64,
        growth_per_day: slope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_794_665_100;
    const TODAY: i64 = NOW - NOW % DAY;
    const GB: f64 = 1e9;

    fn point(day_offset: i64, value: f64) -> Point {
        Point {
            at: TODAY + day_offset * DAY,
            min: value,
            mean: value,
            max: value,
            count: 1,
        }
    }

    /// `days` daily values ending today on the line `end + slope * offset`.
    fn line(days: i64, end: f64, slope: f64) -> Vec<Point> {
        (1 - days..=0)
            .map(|d| point(d, end + slope * d as f64))
            .collect()
    }

    #[test]
    fn rising_line_counts_from_today() {
        // 1000 GB used, +2 GB per day, 1400 GB total: 200 days.
        let f = forecast(&line(30, 1000.0 * GB, 2.0 * GB), 1400.0 * GB, NOW).expect("trend");
        assert!((f.days_to_full - 200.0).abs() < 1e-6, "{}", f.days_to_full);
        assert!((f.growth_per_day - 2.0 * GB).abs() < 1.0);
        assert_eq!(f.full_at, NOW + 200 * DAY);
    }

    #[test]
    fn falling_and_flat_have_no_trend() {
        assert_eq!(
            forecast(&line(30, 1000.0 * GB, -GB), 1400.0 * GB, NOW),
            None
        );
        assert_eq!(
            forecast(&line(30, 1000.0 * GB, 0.0), 1400.0 * GB, NOW),
            None
        );
    }

    #[test]
    fn needs_seven_daily_values() {
        let six = line(6, 1000.0 * GB, 2.0 * GB);
        let seven = line(7, 1000.0 * GB, 2.0 * GB);
        assert_eq!(forecast(&six, 1400.0 * GB, NOW), None);
        assert!(forecast(&seven, 1400.0 * GB, NOW).is_some());
    }

    #[test]
    fn drop_at_the_end_follows_the_line() {
        // 29 days on +2 GB per day, then 100 GB deleted on the last day.
        let mut points = line(30, 1000.0 * GB, 2.0 * GB);
        let last = points.last_mut().expect("last");
        last.mean -= 100.0 * GB;
        let last_value = last.mean;
        let f = forecast(&points, 1400.0 * GB, NOW).expect("trend");
        let naive = (1400.0 * GB - last_value) / f.growth_per_day;
        assert!((f.days_to_full - naive).abs() > 10.0);
        // The line is pulled down only slightly: the level today is far
        // above the last value.
        assert!(f.days_to_full < naive);
        assert!(f.days_to_full > 150.0);
    }

    #[test]
    fn ten_year_limit() {
        // Growth 1 GB per day, 3650 days left, and a bit more or less.
        let end = 1000.0 * GB;
        let at_limit = forecast(&line(30, end, GB), end + 3650.0 * GB, NOW).expect("limit");
        assert!((at_limit.days_to_full - 3650.0).abs() < 1e-6);
        assert_eq!(forecast(&line(30, end, GB), end + 3651.0 * GB, NOW), None);
    }

    #[test]
    fn line_above_total_is_full_today() {
        let f = forecast(&line(30, 1500.0 * GB, 2.0 * GB), 1400.0 * GB, NOW).expect("trend");
        assert_eq!(f.days_to_full, 0.0);
        assert_eq!(f.full_at, NOW);
    }

    #[test]
    fn values_older_than_30_days_do_not_count() {
        let mut points = line(30, 1000.0 * GB, 2.0 * GB);
        points.insert(0, point(-30, 5000.0 * GB));
        points.insert(0, point(-40, 0.0));
        let with = forecast(&points, 1400.0 * GB, NOW).expect("trend");
        let without = forecast(&line(30, 1000.0 * GB, 2.0 * GB), 1400.0 * GB, NOW).expect("trend");
        assert_eq!(with, without);
    }

    #[test]
    fn gaps_keep_the_slope_of_an_exact_line() {
        let mut points = line(30, 1000.0 * GB, 2.0 * GB);
        points.retain(|p| !(TODAY - 12 * DAY..=TODAY - 10 * DAY).contains(&p.at));
        assert_eq!(points.len(), 27);
        let f = forecast(&points, 1400.0 * GB, NOW).expect("trend");
        assert!((f.growth_per_day - 2.0 * GB).abs() < 1.0);
        assert!((f.days_to_full - 200.0).abs() < 1e-6);
    }

    #[test]
    fn unusable_total_has_no_trend() {
        let points = line(30, 1000.0 * GB, 2.0 * GB);
        assert_eq!(forecast(&points, 0.0, NOW), None);
        assert_eq!(forecast(&points, f64::NAN, NOW), None);
    }
}
