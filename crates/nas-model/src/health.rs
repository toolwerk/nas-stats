//! Health levels and small derived values shared by all types.

use serde::{Deserialize, Serialize};

/// Health as the vendor reports it, reduced to three levels.
///
/// Ordered from good to bad, so `max` yields the worst level.
/// `Option<Health>` with `None` stands for "unknown".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Warning,
    Critical,
}

impl Health {
    /// Worst known level, `None` when every value is unknown or the input is empty.
    pub fn worst<I>(levels: I) -> Option<Health>
    where
        I: IntoIterator<Item = Option<Health>>,
    {
        levels.into_iter().flatten().max()
    }
}

/// Share of `part` in `total` in percent.
///
/// `None` when `total` is zero. The result is not capped, so values above
/// 100 show an inconsistent vendor answer instead of hiding it.
pub fn percent(part: u64, total: u64) -> Option<f64> {
    if total == 0 {
        return None;
    }
    Some(part as f64 / total as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_of_a_quarter() {
        assert_eq!(percent(250, 1000), Some(25.0));
    }

    #[test]
    fn percent_without_total_is_none() {
        assert_eq!(percent(250, 0), None);
    }

    #[test]
    fn percent_is_not_capped() {
        assert_eq!(percent(1500, 1000), Some(150.0));
    }

    #[test]
    fn worst_picks_the_highest_level() {
        let levels = [
            Some(Health::Ok),
            None,
            Some(Health::Critical),
            Some(Health::Warning),
        ];
        assert_eq!(Health::worst(levels), Some(Health::Critical));
    }

    #[test]
    fn worst_of_unknown_is_none() {
        assert_eq!(Health::worst([None, None]), None);
        assert_eq!(Health::worst([]), None);
    }
}
