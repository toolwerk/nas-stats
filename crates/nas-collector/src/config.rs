//! The configuration file of the collector, TOML:
//!
//! ```toml
//! listen = "0.0.0.0:8743"     # optional
//! interval_minutes = 5        # optional, at least 5
//!
//! [[nas]]
//! id = "keller"
//! provider = "qnap"
//! base_url = "https://192.168.1.20:443"
//! fingerprint = "AB:CD:…"
//! user = "nasstats"
//! password = "…"
//!
//! [[nas]]
//! id = "tower"
//! provider = "unraid"
//! base_url = "https://192.168.1.30"
//! fingerprint = "AB:CD:…"
//! api_key = "…"
//! ```
//!
//! A QNAP needs `user` and `password`, an Unraid only `api_key`. An error
//! names the field, never the password or the key.

use std::fmt;
use std::net::SocketAddr;
use std::path::Path;

use nas_http::Fingerprint;
use nas_model::ProviderError;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Where the configuration is read when `NAS_COLLECTOR_CONFIG` is not set.
pub const DEFAULT_PATH: &str = "/config/collector.toml";
/// Address of the interface when the file names none.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8743";
/// Shortest and default interval, minutes.
pub const MIN_INTERVAL_MINUTES: u32 = 5;

/// A checked configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub listen: SocketAddr,
    pub interval_minutes: u32,
    pub nas: Vec<NasConfig>,
}

/// One NAS the collector queries.
#[derive(Clone, PartialEq)]
pub struct NasConfig {
    /// Id of the NAS at the collector: letters, digits, `.`, `_`, `-`.
    pub id: String,
    pub provider: nas_model::ProviderKind,
    pub base_url: String,
    pub fingerprint: Fingerprint,
    /// User name; empty for Unraid.
    pub user: String,
    /// Password; for Unraid the API key.
    pub password: String,
}

impl fmt::Debug for NasConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NasConfig")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("fingerprint", &self.fingerprint)
            .field("user", &self.user)
            .field("password", &"…")
            .finish()
    }
}

impl NasConfig {
    /// SHA-256 over user, password, address and fingerprint, hex. Changes
    /// when the user changes the access, which lifts a block.
    pub fn access(&self) -> String {
        let mut hash = Sha256::new();
        for part in [
            self.user.as_str(),
            self.password.as_str(),
            self.base_url.as_str(),
            &self.fingerprint.to_string(),
        ] {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Why the configuration cannot be used; the text names the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: Option<String>,
    interval_minutes: Option<i64>,
    #[serde(default)]
    nas: Vec<RawNas>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNas {
    id: String,
    provider: String,
    base_url: String,
    fingerprint: String,
    user: Option<String>,
    password: Option<String>,
    api_key: Option<String>,
}

/// Reads and checks the file at `path`.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: nicht lesbar: {e}", path.display())))?;
    parse(&text).map_err(|e| ConfigError(format!("{}: {e}", path.display())))
}

/// Checks a configuration given as text.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| syntax_error(text, &e))?;
    let listen = raw.listen.as_deref().unwrap_or(DEFAULT_LISTEN);
    let listen: SocketAddr = listen
        .parse()
        .map_err(|_| ConfigError(format!("listen: keine Adresse mit Port: {listen:?}")))?;
    let interval_minutes = match raw.interval_minutes {
        None => MIN_INTERVAL_MINUTES,
        Some(minutes) if minutes < i64::from(MIN_INTERVAL_MINUTES) => {
            return Err(ConfigError(format!(
                "interval_minutes: mindestens {MIN_INTERVAL_MINUTES}, eingetragen {minutes}"
            )));
        }
        Some(minutes) => u32::try_from(minutes)
            .map_err(|_| ConfigError(format!("interval_minutes: zu groß: {minutes}")))?,
    };
    if raw.nas.is_empty() {
        return Err(ConfigError(
            "nas: kein NAS eingetragen ([[nas]] fehlt)".into(),
        ));
    }
    let mut nas: Vec<NasConfig> = Vec::new();
    for entry in raw.nas {
        let checked = check_nas(entry)?;
        if nas.iter().any(|n| n.id == checked.id) {
            return Err(ConfigError(format!(
                "nas.id: {:?} steht doppelt in der Datei",
                checked.id
            )));
        }
        nas.push(checked);
    }
    Ok(Config {
        listen,
        interval_minutes,
        nas,
    })
}

fn check_nas(raw: RawNas) -> Result<NasConfig, ConfigError> {
    let valid_id = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if raw.id.is_empty() || !raw.id.chars().all(valid_id) {
        return Err(ConfigError(format!(
            "nas.id: nur Buchstaben, Ziffern, '.', '_' und '-': {:?}",
            raw.id
        )));
    }
    let field = |name: &str| format!("nas {:?}: {name}", raw.id);
    let provider = match raw.provider.as_str() {
        "qnap" => nas_model::ProviderKind::Qnap,
        "unraid" => nas_model::ProviderKind::Unraid,
        other => {
            return Err(ConfigError(format!(
                "{}: nur \"qnap\" und \"unraid\" werden unterstützt, eingetragen {other:?}",
                field("provider")
            )));
        }
    };
    if !raw.base_url.starts_with("https://") {
        return Err(ConfigError(format!(
            "{}: nur https:// ist erlaubt",
            field("base_url")
        )));
    }
    let fingerprint: Fingerprint = raw
        .fingerprint
        .parse()
        .map_err(|e: ProviderError| ConfigError(format!("{}: {e}", field("fingerprint"))))?;
    let filled = |name: &str, value: Option<String>| match value {
        None => Err(ConfigError(format!("{}: fehlt", field(name)))),
        Some(value) if value.is_empty() => Err(ConfigError(format!("{}: leer", field(name)))),
        Some(value) => Ok(value),
    };
    let unused = |name: &str, value: &Option<String>, provider: &str| match value {
        Some(_) => Err(ConfigError(format!(
            "{}: gilt nicht für {provider}",
            field(name)
        ))),
        None => Ok(()),
    };
    let (user, password) = match provider {
        nas_model::ProviderKind::Unraid => {
            unused("user", &raw.user, "unraid")?;
            unused("password", &raw.password, "unraid")?;
            (String::new(), filled("api_key", raw.api_key)?)
        }
        _ => {
            unused("api_key", &raw.api_key, "qnap")?;
            (filled("user", raw.user)?, filled("password", raw.password)?)
        }
    };
    Ok(NasConfig {
        id: raw.id,
        provider,
        base_url: raw.base_url,
        fingerprint,
        user,
        password,
    })
}

/// A TOML error as line and message, without the text of the file. Only
/// messages that quote a field name pass; any other may quote a value,
/// e.g. a password, and becomes a plain "invalid".
fn syntax_error(text: &str, e: &toml::de::Error) -> ConfigError {
    let message = e.message();
    let names_a_field = ["unknown field", "missing field", "duplicate key"]
        .iter()
        .any(|start| message.starts_with(start));
    let message = if names_a_field {
        message
    } else {
        "ungültiger Wert oder ungültige Schreibweise"
    };
    match e.span() {
        Some(span) => {
            let start = span.start.min(text.len());
            let line_no = text[..start].matches('\n').count() + 1;
            ConfigError(format!("Zeile {line_no}: {message}"))
        }
        None => ConfigError(format!("TOML: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:\
                      AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";

    fn nas(id: &str) -> String {
        format!(
            "[[nas]]\nid = \"{id}\"\nprovider = \"qnap\"\nbase_url = \"https://192.0.2.10\"\n\
             fingerprint = \"{FP}\"\nuser = \"nasstats\"\npassword = \"geheim-123\"\n"
        )
    }

    fn error(text: &str) -> String {
        parse(text).expect_err("config is refused").0
    }

    #[test]
    fn minimal_file_gets_the_defaults() {
        let config = parse(&nas("keller")).expect("config parses");
        assert_eq!(config.listen, DEFAULT_LISTEN.parse().expect("address"));
        assert_eq!(config.interval_minutes, 5);
        assert_eq!(config.nas.len(), 1);
        let keller = &config.nas[0];
        assert_eq!(keller.id, "keller");
        assert_eq!(keller.provider, nas_model::ProviderKind::Qnap);
        assert_eq!(keller.fingerprint.to_string(), FP);
        assert!(!format!("{keller:?}").contains("geheim"));
    }

    #[test]
    fn example_file_parses() {
        let text = include_str!("../../../docker/collector/collector.example.toml");
        let config = parse(text).expect("example parses");
        assert_eq!(config.interval_minutes, 5);
        assert_eq!(config.nas[0].id, "keller");
        // The commented Unraid entry works once the hashes are gone.
        let uncommented: String = text
            .lines()
            .map(|line| match line.strip_prefix("# ") {
                Some(rest)
                    if rest == "[[nas]]"
                        || rest.split_once(" = ").is_some_and(|(key, _)| {
                            key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                        }) =>
                {
                    rest
                }
                _ => line,
            })
            .map(|line| format!("{line}\n"))
            .collect();
        let config = parse(&uncommented).expect("example with Unraid parses");
        assert_eq!(config.nas.len(), 2);
        assert_eq!(config.nas[1].provider, nas_model::ProviderKind::Unraid);
        assert_eq!(config.nas[1].password, "hier-der-api-key");
    }

    #[test]
    fn listen_and_interval_are_taken() {
        let text = format!(
            "listen = \"127.0.0.1:9000\"\ninterval_minutes = 15\n{}",
            nas("a")
        );
        let config = parse(&text).expect("config parses");
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.interval_minutes, 15);
    }

    #[test]
    fn errors_name_the_field() {
        assert!(
            error(&format!("interval_minutes = 4\n{}", nas("a"))).starts_with("interval_minutes")
        );
        assert!(error(&format!("listen = \"irgendwo\"\n{}", nas("a"))).starts_with("listen"));
        assert!(error("listen = \"0.0.0.0:8743\"\n").starts_with("nas:"));
        assert!(error(&format!("{}{}", nas("a"), nas("a"))).contains("doppelt"));
        assert!(error(&nas("a b")).starts_with("nas.id"));
        let synology = nas("a").replace("\"qnap\"", "\"synology\"");
        assert!(error(&synology).contains("provider"));
        let http = nas("a").replace("https://", "http://");
        assert!(error(&http).contains("base_url"));
        let short = nas("a").replace(FP, "AB:CD");
        assert!(error(&short).contains("fingerprint"));
        let empty = nas("a").replace("\"nasstats\"", "\"\"");
        assert!(error(&empty).contains("user"));
        let unknown = format!("{}port = 8080\n", nas("a"));
        assert!(error(&unknown).contains("port"), "{}", error(&unknown));
        let missing = nas("a").replace("user = \"nasstats\"\n", "");
        assert!(error(&missing).contains("user"));
    }

    fn unraid(id: &str) -> String {
        format!(
            "[[nas]]\nid = \"{id}\"\nprovider = \"unraid\"\nbase_url = \"https://192.0.2.30\"\n\
             fingerprint = \"{FP}\"\napi_key = \"schluessel-123\"\n"
        )
    }

    #[test]
    fn unraid_needs_only_the_api_key() {
        let config = parse(&unraid("tower")).expect("config parses");
        let tower = &config.nas[0];
        assert_eq!(tower.provider, nas_model::ProviderKind::Unraid);
        assert_eq!(tower.user, "");
        assert_eq!(tower.password, "schluessel-123");
        assert!(!format!("{tower:?}").contains("schluessel"));

        let missing = unraid("a").replace("api_key = \"schluessel-123\"\n", "");
        assert!(
            error(&missing).contains("api_key: fehlt"),
            "{}",
            error(&missing)
        );
        let empty = unraid("a").replace("\"schluessel-123\"", "\"\"");
        assert!(error(&empty).contains("api_key: leer"));
        let with_user = format!("{}user = \"admin\"\n", unraid("a"));
        assert!(error(&with_user).contains("user"));
        let with_password = format!("{}password = \"x\"\n", unraid("a"));
        assert!(error(&with_password).contains("password"));
        let qnap_with_key = format!("{}api_key = \"x\"\n", nas("a"));
        assert!(error(&qnap_with_key).contains("api_key"));
        let qnap_without_password = nas("a").replace("password = \"geheim-123\"\n", "");
        assert!(error(&qnap_without_password).contains("password: fehlt"));
        for text in [missing, empty, with_user, with_password] {
            assert!(!error(&text).contains("schluessel"), "{}", error(&text));
        }
    }

    #[test]
    fn errors_never_show_the_password() {
        let cases = [
            nas("a").replace("password = \"geheim-123\"", "password = geheim-123"),
            nas("a").replace("password = \"geheim-123\"", "password = 1234567"),
            nas("a").replace("password = \"geheim-123\"", "password = \"\""),
            format!("{}\nunbekannt = 1\n", nas("a")),
            nas("a").replace("password = \"geheim-123\"", "\"password\" = 1234567"),
            nas("a").replace(
                "password = \"geheim-123\"",
                "user2 = 1\npassword = [\"geheim-123\"]",
            ),
            format!(
                "nas = [{{ id = \"a\", provider = \"qnap\", base_url = \"https://x\", \
                 fingerprint = \"{FP}\", user = \"u\", password = 1234567 }}]"
            ),
        ];
        for text in cases {
            let message = error(&text);
            assert!(!message.contains("geheim"), "{message}");
            assert!(!message.contains("1234567"), "{message}");
        }
    }

    #[test]
    fn access_changes_with_every_part() {
        let base = parse(&nas("a")).expect("config parses").nas[0].clone();
        let mut seen = vec![base.access()];
        let mut other = base.clone();
        other.password = "anders".into();
        seen.push(other.access());
        let mut other = base.clone();
        other.user = "admin".into();
        seen.push(other.access());
        let mut other = base.clone();
        other.base_url = "https://192.0.2.11".into();
        seen.push(other.access());
        let mut other = base.clone();
        other.fingerprint = Fingerprint::of_der(b"x");
        seen.push(other.access());
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 5);
        assert_eq!(base.access(), base.clone().access());
        assert_eq!(base.access().len(), 64);
    }
}
