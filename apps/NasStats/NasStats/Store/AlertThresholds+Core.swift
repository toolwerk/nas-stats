import Foundation

extension AlertThresholds {
    /// The thresholds as the core takes them (`NasCore.setThresholds`,
    /// `demoSnapshots`); the backup limit stays in the core.
    var core: Thresholds {
        Thresholds(
            diskTempWarningC: diskTempWarningC,
            fillWarningPercent: fill.warning,
            fillCriticalPercent: fill.critical
        )
    }
}
