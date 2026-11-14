//! Single measurements for the history.

use serde::{Deserialize, Serialize};

/// What a [`Sample`] measures. The unit follows from the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metric {
    /// Used space of a volume, in bytes.
    #[serde(rename = "volume.used")]
    VolumeUsed,
    /// Total space of a volume, in bytes.
    #[serde(rename = "volume.total")]
    VolumeTotal,
    /// Disk temperature, in degrees Celsius.
    #[serde(rename = "disk.temp")]
    DiskTemp,
    /// CPU load, in percent.
    #[serde(rename = "cpu")]
    Cpu,
    /// Used memory, in percent.
    #[serde(rename = "mem")]
    Mem,
    /// Received bytes per second of an interface.
    #[serde(rename = "net.rx")]
    NetRx,
    /// Sent bytes per second of an interface.
    #[serde(rename = "net.tx")]
    NetTx,
    /// System temperature, in degrees Celsius.
    #[serde(rename = "sys.temp")]
    SysTemp,
}

impl Metric {
    /// Name as stored and serialized, e.g. `disk.temp`.
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::VolumeUsed => "volume.used",
            Metric::VolumeTotal => "volume.total",
            Metric::DiskTemp => "disk.temp",
            Metric::Cpu => "cpu",
            Metric::Mem => "mem",
            Metric::NetRx => "net.rx",
            Metric::NetTx => "net.tx",
            Metric::SysTemp => "sys.temp",
        }
    }
}

/// One measured value of a NAS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Local id of the [`crate::Nas`].
    pub nas_id: String,
    pub metric: Metric,
    /// Volume, disk or interface id; `None` for values of the whole NAS.
    pub key: Option<String>,
    /// Time of the measurement, Unix seconds UTC.
    pub at: i64,
    /// Value in the unit of [`Sample::metric`].
    pub value: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Metric; 8] = [
        Metric::VolumeUsed,
        Metric::VolumeTotal,
        Metric::DiskTemp,
        Metric::Cpu,
        Metric::Mem,
        Metric::NetRx,
        Metric::NetTx,
        Metric::SysTemp,
    ];

    #[test]
    fn disk_temp_name() {
        assert_eq!(Metric::DiskTemp.as_str(), "disk.temp");
    }

    #[test]
    fn as_str_matches_serde_name() {
        for metric in ALL {
            let json = serde_json::to_string(&metric).expect("metric serializes");
            assert_eq!(json, format!("\"{}\"", metric.as_str()));
        }
    }
}
