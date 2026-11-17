//! Anonymisation of the recorded answers. Structure, numbers and the
//! real shape of the values stay, so the parsers see what the NAS sends;
//! serial numbers, names, addresses and secrets are replaced.
//!
//! Two kinds of rules:
//! - by key (XML element, XML attribute, JSON key, case ignored), see
//!   [`classify`];
//! - in every text: IPv4, IPv6 and MAC addresses, and every real value
//!   replaced by a key rule (from 3 characters on), so a serial number
//!   also disappears where it shows up under another key.
//!
//! [`Anonymizer::learn`] should see every file before
//! [`Anonymizer::anonymize`] writes the first one, so a value found late
//! is replaced everywhere. [`Anonymizer::leaks`] checks the result.

use std::collections::BTreeMap;
use std::net::Ipv6Addr;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesCData, BytesPI, BytesRef, BytesStart, BytesText, Event};
use regex::Regex;
use serde_json::Value;

/// What a key says about its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Serial,
    Host,
    /// Session ids, tokens, passwords: always replaced.
    Secret,
    /// User names, mail, DDNS, domain, workgroup: replaced unless the
    /// value is a flag or a small number (`ddns_enable` = `1`).
    Personal,
    Volume,
    Share,
    Alias,
    Container,
}

pub const REDACTED: &str = "REDACTED";
pub const DEMO_HOST: &str = "nas-demo";

/// Values that are switches, not names. They stay.
const FLAG_WORDS: &[&str] = &[
    "yes", "no", "true", "false", "on", "off", "none", "null", "n/a", "ok", "unknown", "enable",
    "enabled", "disable", "disabled", "-", "--",
];

/// Keys that describe a setting (`userType`, `session_timeout`,
/// `ddns_enable`) rather than hold a name or secret. Password-like keys
/// never count as such.
fn is_descriptive(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ![
        "pwd",
        "password",
        "token",
        "sid",
        "jwt",
        "cookie",
        "serial",
        "cuid",
        "ddns",
        "domain",
        "workgroup",
        "email",
        "community",
    ]
    .iter()
    .any(|s| k.contains(s))
        && [
            "type", "timeout", "enable", "enabled", "expiry", "count", "support", "status",
        ]
        .iter()
        .any(|s| k.ends_with(s))
}

/// A setting value: one short word (`local`, `1800`, `ldap_ad`), no
/// name with dots, spaces or `@`. Anything else under a descriptive key
/// is still treated by the key rules.
fn is_setting_value(value: &str) -> bool {
    let v = value.trim();
    v.len() <= 12 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Classifies a key. `json` adds the Docker-style `name`/`names` of
/// Container Station.
pub fn classify(key: &str, json: bool) -> Option<Kind> {
    let k = key.to_ascii_lowercase();
    let has = |s: &str| k.contains(s);
    if has("serial") {
        Some(Kind::Serial)
    } else if ["hostname", "server_name", "servername", "nas_name"].contains(&k.as_str()) {
        Some(Kind::Host)
    } else if k == "authsid"
        || k == "suid"
        || k == "sid"
        || [
            "token",
            "pwd",
            "password",
            "cuid",
            "session",
            "jwt",
            "cookie",
            "community",
        ]
        .iter()
        .any(|s| has(s))
    {
        Some(Kind::Secret)
    } else if ["user", "email", "ddns", "domain", "workgroup"]
        .iter()
        .any(|s| has(s))
    {
        Some(Kind::Personal)
    } else if k == "volumelabel" || has("pool_name") || has("poolname") {
        Some(Kind::Volume)
    } else if has("share") || has("folder_name") || has("foldername") {
        Some(Kind::Share)
    } else if has("alias") {
        Some(Kind::Alias)
    } else if json && (k == "name" || k == "names") {
        Some(Kind::Container)
    } else {
        None
    }
}

/// Key for the text of the innermost element. A generic `<name>` or
/// `<label>` inside a pool, volume or share element takes the meaning
/// of that element (`<pool><name>Fotos</name></pool>`).
fn context_key(stack: &[String]) -> Option<String> {
    let (last, parents) = stack.split_last()?;
    let generic = ["name", "label", "displayname", "display_name", "alias"]
        .contains(&last.to_ascii_lowercase().as_str());
    if generic {
        for parent in parents.iter().rev().take(2) {
            let p = parent.to_ascii_lowercase();
            if p.contains("pool") || p.contains("volume") {
                return Some("pool_name".to_owned());
            }
            if p.contains("share") || p.contains("folder") {
                return Some("sharename".to_owned());
            }
        }
    }
    Some(last.clone())
}

/// Empty values, flags and small numbers say nothing about the device.
fn is_neutral(value: &str) -> bool {
    let v = value.trim();
    v.is_empty()
        || FLAG_WORDS.contains(&v.to_ascii_lowercase().as_str())
        || (v.len() <= 4 && v.chars().all(|c| c.is_ascii_digit()))
}

/// `A`, `B`, …, `Z`, `AA`, `AB`, …
fn letters(mut n: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn ipv4_parts(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 3 || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out[i] = p.parse().ok()?;
    }
    Some(out)
}

/// Addresses that identify nothing, plus the documentation ranges used
/// as replacement.
fn ipv4_allowed(o: [u8; 4]) -> bool {
    o == [0, 0, 0, 0]
        || o == [127, 0, 0, 1]
        || o[0] == 255
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
}

fn ipv6_allowed(a: &Ipv6Addr) -> bool {
    let s = a.segments();
    a.is_loopback() || a.is_unspecified() || (s[0] == 0x2001 && s[1] == 0x0db8)
}

fn mac_allowed(mac: &str) -> bool {
    let m = mac.to_ascii_lowercase().replace('-', ":");
    m == "00:00:00:00:00:00" || m == "ff:ff:ff:ff:ff:ff" || m.starts_with("02:00:00:00:00:")
}

/// Trims the edges of an IPv6 candidate run (`dev:fe80::1:` → `fe80::1`)
/// and returns the offsets of the address inside the run.
fn ipv6_in_run(run: &str) -> Option<(usize, usize, Ipv6Addr)> {
    let mut start = 0;
    let mut end = run.len();
    if run.starts_with(':') && !run.starts_with("::") {
        start = 1;
    }
    while end > start && run[..end].ends_with('.') {
        end -= 1;
    }
    if run[start..end].ends_with(':') && !run[start..end].ends_with("::") {
        end -= 1;
    }
    let candidate = &run[start..end];
    if candidate.matches(':').count() < 2 {
        return None;
    }
    candidate.parse().ok().map(|a| (start, end, a))
}

fn is_word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric())
}

/// Names QTS gives every device (shares, volume). They identify
/// nothing and also appear in system paths, so the text pass leaves
/// them alone; under their key they are still replaced.
const DEFAULT_NAMES: &[&str] = &[
    "data",
    "public",
    "web",
    "multimedia",
    "download",
    "recordings",
    "homes",
    "home",
    "container",
    "usb",
    "admin",
    "administrators",
    "everyone",
    "guest",
];

struct Patterns {
    ipv4_run: Regex,
    ipv6_run: Regex,
    mac: Regex,
    snmp_system: Regex,
}

impl Patterns {
    fn new() -> Self {
        let re = |s: &str| Regex::new(s).unwrap_or_else(|e| panic!("pattern {s}: {e}"));
        Self {
            ipv4_run: re(r"[0-9][0-9.]*[0-9]"),
            ipv6_run: re(r"[0-9A-Fa-f:.]*:[0-9A-Fa-f:.]*"),
            mac: re(r"\b[0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}(?:[:-][0-9A-Fa-f]{2}){4}\b"),
            snmp_system: re(
                r"(?m)^(.*\b(?:sysName|sysContact|sysLocation)\b[^=\n]*=\s*(?:STRING:\s*)?)(.*?)\r?$",
            ),
        }
    }
}

pub struct Anonymizer {
    maps: BTreeMap<String, BTreeMap<String, String>>,
    /// Real value (lower case) → replacement, for the text pass.
    known: BTreeMap<String, String>,
    known_re: Option<(usize, Regex)>,
    patterns: Patterns,
}

impl Default for Anonymizer {
    fn default() -> Self {
        Self::new()
    }
}

/// One piece of an XML text node, kept as read.
enum Piece {
    /// Escaped text.
    Text(String),
    /// Entity or character reference without `&` and `;`.
    Ref(String),
    CData(String),
}

impl Anonymizer {
    pub fn new() -> Self {
        Self {
            maps: BTreeMap::new(),
            known: BTreeMap::new(),
            known_re: None,
            patterns: Patterns::new(),
        }
    }

    /// Adds a real value (host, user name) that must not appear in any
    /// result. Shorter than 3 characters is ignored, it would hit
    /// everywhere.
    pub fn add_known(&mut self, real: &str, replacement: &str) {
        let real = real.trim();
        if real.chars().count() >= 3
            && !is_neutral(real)
            && real != replacement
            && !DEFAULT_NAMES.contains(&real.to_lowercase().as_str())
        {
            self.known
                .entry(real.to_lowercase())
                .or_insert_with(|| replacement.to_owned());
        }
    }

    /// Adds a secret of this run (password, session id), even if it
    /// looks like a flag or a small number.
    pub fn add_secret(&mut self, real: &str) {
        if real.chars().count() >= 3 {
            self.known
                .entry(real.to_lowercase())
                .or_insert_with(|| REDACTED.to_owned());
        }
    }

    /// Consistent replacement from a numbered family.
    fn numbered(&mut self, family: &str, value: &str, make: impl Fn(usize) -> String) -> String {
        let map = self.maps.entry(family.to_owned()).or_default();
        let n = map.len();
        map.entry(value.to_owned())
            .or_insert_with(|| make(n))
            .clone()
    }

    /// Replacement for a value under a classified key, `None` to keep it.
    fn map_value(&mut self, kind: Kind, value: &str) -> Option<String> {
        // A secret is replaced whatever it looks like.
        if value.trim().is_empty() || (kind != Kind::Secret && is_neutral(value)) {
            return None;
        }
        let replacement = match kind {
            Kind::Serial => self.numbered("serial", value, |n| format!("SERIAL-{}", letters(n))),
            Kind::Host => DEMO_HOST.to_owned(),
            Kind::Secret | Kind::Personal => REDACTED.to_owned(),
            Kind::Volume => self.numbered("volume", value, |n| format!("Volume {}", n + 1)),
            Kind::Share => self.numbered("share", value, |n| format!("Share {}", n + 1)),
            Kind::Alias => self.numbered("alias", value, |n| format!("Alias {}", n + 1)),
            Kind::Container => {
                // Docker prefixes names with `/`; keep the prefix.
                let (prefix, bare) = value
                    .strip_prefix('/')
                    .map_or(("", value), |rest| ("/", rest));
                let name = self.numbered("container", bare, |n| format!("container-{}", n + 1));
                self.add_known(bare, &name);
                format!("{prefix}{name}")
            }
        };
        self.add_known(value, &replacement);
        Some(replacement)
    }

    fn known_regex(&self) -> Option<Regex> {
        if self.known.is_empty() {
            return None;
        }
        let mut values: Vec<&String> = self.known.keys().collect();
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        let alternation = values
            .iter()
            .map(|v| regex::escape(v))
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&format!("(?i)(?:{alternation})")).ok()
    }

    /// Matches of known values that stand alone (no letter or digit
    /// directly before or after, so `kellernas_snap` is a hit), as
    /// `(start, end, replacement)`.
    ///
    /// Replacements already in the text are skipped: a share called
    /// `Container` must not hit `container-1`, and `REDACTED` is no leak.
    fn known_matches(&self, re: &Regex, text: &str) -> Vec<(usize, usize, String)> {
        let protected = self.replacement_ranges(text);
        re.find_iter(text)
            .filter(|m| {
                !is_word_char(text[..m.start()].chars().next_back())
                    && !is_word_char(text[m.end()..].chars().next())
            })
            .filter(|m| {
                !protected
                    .iter()
                    .any(|(s, e)| m.start() < *e && *s < m.end())
            })
            .filter_map(|m| {
                self.known
                    .get(&m.as_str().to_lowercase())
                    .map(|r| (m.start(), m.end(), r.clone()))
            })
            .collect()
    }

    /// Where replacement values (`Share 1`, `container-2`, `REDACTED`)
    /// stand in `text`.
    fn replacement_ranges(&self, text: &str) -> Vec<(usize, usize)> {
        let mut replacements: Vec<&str> = self
            .known
            .values()
            .map(String::as_str)
            .chain(
                self.maps
                    .values()
                    .flat_map(|m| m.values().map(String::as_str)),
            )
            .chain([REDACTED, DEMO_HOST])
            .collect();
        replacements.sort_by_key(|r| std::cmp::Reverse(r.len()));
        replacements.dedup();
        let alternation = replacements
            .iter()
            .map(|r| regex::escape(r))
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&alternation)
            .map(|re| re.find_iter(text).map(|m| (m.start(), m.end())).collect())
            .unwrap_or_default()
    }

    fn replace_known(&mut self, text: &str) -> String {
        let len = self.known.len();
        if self.known_re.as_ref().is_none_or(|(n, _)| *n != len) {
            self.known_re = self.known_regex().map(|re| (len, re));
        }
        let Some((_, re)) = &self.known_re else {
            return text.to_owned();
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (start, end, replacement) in self.known_matches(re, text) {
            out.push_str(&text[last..start]);
            out.push_str(&replacement);
            last = end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// IPv6 addresses in `text` as `(start, end, address)`, only where
    /// no letter or digit touches them (`MIB::sys` is no address).
    fn ipv6_matches(&self, text: &str) -> Vec<(usize, usize, Ipv6Addr)> {
        self.patterns
            .ipv6_run
            .find_iter(text)
            .filter_map(|m| {
                let (s, e, addr) = ipv6_in_run(m.as_str())?;
                let (start, end) = (m.start() + s, m.start() + e);
                let free = !is_word_char(text[..start].chars().next_back())
                    && !is_word_char(text[end..].chars().next());
                free.then_some((start, end, addr))
            })
            .collect()
    }

    fn replace_ipv6(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (start, end, addr) in self.ipv6_matches(text) {
            if ipv6_allowed(&addr) {
                continue;
            }
            let replacement = self.numbered("ipv6", &addr.to_string(), |n| {
                format!("2001:db8::{:x}", n + 1)
            });
            out.push_str(&text[last..start]);
            out.push_str(&replacement);
            last = end;
        }
        out.push_str(&text[last..]);
        out
    }

    fn replace_mac(&mut self, text: &str) -> String {
        let found: Vec<(usize, usize, String)> = self
            .patterns
            .mac
            .find_iter(text)
            .map(|m| (m.start(), m.end(), m.as_str().to_owned()))
            .collect();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (start, end, mac) in found {
            if mac_allowed(&mac) {
                continue;
            }
            let norm = mac.to_ascii_lowercase().replace('-', ":");
            let replacement = self.numbered("mac", &norm, |n| {
                format!("02:00:00:00:00:{:02x}", (n + 1) % 256)
            });
            out.push_str(&text[last..start]);
            out.push_str(&replacement);
            last = end;
        }
        out.push_str(&text[last..]);
        out
    }

    fn replace_ipv4(&mut self, text: &str) -> String {
        let found: Vec<(usize, usize)> = self
            .patterns
            .ipv4_run
            .find_iter(text)
            .map(|m| (m.start(), m.end()))
            .collect();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (start, end) in found {
            let run = &text[start..end];
            let Some(octets) = ipv4_parts(run) else {
                continue;
            };
            if ipv4_allowed(octets) {
                continue;
            }
            let replacement = self.numbered("ipv4", run, |n| {
                if n < 254 {
                    format!("192.0.2.{}", n + 1)
                } else {
                    format!("198.51.100.{}", (n - 254) % 254 + 1)
                }
            });
            out.push_str(&text[last..start]);
            out.push_str(&replacement);
            last = end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// The text pass: known values, then IPv6, MAC and IPv4 addresses.
    pub fn scrub(&mut self, text: &str) -> String {
        let text = self.replace_known(text);
        let text = self.replace_ipv6(&text);
        let text = self.replace_mac(&text);
        self.replace_ipv4(&text)
    }

    /// Value under a key: key rule first, then the text pass.
    fn value(&mut self, key: Option<&str>, value: &str, json: bool) -> String {
        let mapped = key
            .filter(|k| !(is_descriptive(k) && is_setting_value(value)))
            .and_then(|k| classify(k, json))
            .and_then(|kind| self.map_value(kind, value));
        self.scrub(mapped.as_deref().unwrap_or(value))
    }

    /// Runs the rules over a file without keeping the result, so every
    /// real value is known before the first file is written.
    pub fn learn(&mut self, file_name: &str, content: &str) {
        let _ = self.anonymize(file_name, content);
    }

    /// Anonymises one file, chosen by extension (`.xml`, `.json`, other
    /// text). Returns the new content and the text values in it, which
    /// is what [`Self::leaks`] should check (markup is left out, so a
    /// tag name equal to a share name does not count).
    pub fn anonymize(&mut self, file_name: &str, content: &str) -> (String, String) {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".xml")
            && let Ok(done) = self.xml(content)
        {
            return done;
        }
        if lower.ends_with(".json")
            && let Ok(mut value) = serde_json::from_str::<Value>(content)
        {
            let mut values = String::new();
            self.json_value(None, &mut value, &mut values);
            let mut out = serde_json::to_string_pretty(&value).unwrap_or_default();
            out.push('\n');
            return (out, values);
        }
        let out = self.text(content);
        (out.clone(), out)
    }

    /// Plain text (SNMP walk, broken XML or JSON): system name, contact
    /// and location lines are blanked, then the text pass.
    fn text(&mut self, content: &str) -> String {
        let mut learned = Vec::new();
        let blanked = self
            .patterns
            .snmp_system
            .replace_all(content, |c: &regex::Captures<'_>| {
                let value = c[2].trim().trim_matches('"').to_owned();
                if !value.is_empty() {
                    learned.push(value);
                }
                format!("{}{REDACTED}", &c[1])
            })
            .into_owned();
        for value in learned {
            self.add_known(&value, REDACTED);
        }
        self.scrub(&blanked)
    }

    fn json_value(&mut self, key: Option<&str>, value: &mut Value, values: &mut String) {
        match value {
            Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    values.push_str(k);
                    values.push('\n');
                    self.json_value(Some(k), v, values);
                }
            }
            // `names: ["/a", "/b"]` keeps the key for every item.
            Value::Array(items) => {
                for item in items {
                    self.json_value(key, item, values);
                }
            }
            Value::String(s) => {
                *s = self.value(key, s, true);
                values.push_str(s);
                values.push('\n');
            }
            Value::Number(_) | Value::Bool(_) | Value::Null => {}
        }
    }

    fn xml(&mut self, input: &str) -> Result<(String, String), String> {
        let mut reader = Reader::from_str(input);
        let mut writer = Writer::new(Vec::new());
        let mut stack: Vec<String> = Vec::new();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut values = String::new();
        let io = |e: std::io::Error| e.to_string();
        loop {
            let event = reader.read_event().map_err(|e| e.to_string())?;
            match event {
                Event::Text(t) => pieces.push(Piece::Text(t.into_inner().into_owned())),
                Event::GeneralRef(r) => pieces.push(Piece::Ref(r.into_inner().into_owned())),
                Event::CData(c) => pieces.push(Piece::CData(c.into_inner().into_owned())),
                other => {
                    let key = context_key(&stack);
                    self.flush(&mut writer, key.as_deref(), &mut pieces, &mut values)?;
                    match other {
                        Event::Start(s) => {
                            stack.push(s.name().as_ref().to_owned());
                            let s = self.attributes(s, &mut values)?;
                            writer.write_event(Event::Start(s)).map_err(io)?;
                        }
                        Event::Empty(s) => {
                            let s = self.attributes(s, &mut values)?;
                            writer.write_event(Event::Empty(s)).map_err(io)?;
                        }
                        Event::End(e) => {
                            stack.pop();
                            writer.write_event(Event::End(e)).map_err(io)?;
                        }
                        Event::Eof => break,
                        // Comments and the like are text as well.
                        Event::Comment(t) => {
                            let t = self.scrub_markup(&t.into_inner(), &mut values);
                            writer
                                .write_event(Event::Comment(BytesText::from_escaped(t)))
                                .map_err(io)?;
                        }
                        Event::DocType(t) => {
                            let t = self.scrub_markup(&t.into_inner(), &mut values);
                            writer
                                .write_event(Event::DocType(BytesText::from_escaped(t)))
                                .map_err(io)?;
                        }
                        Event::PI(p) => {
                            let t = self.scrub_markup(&p.into_inner(), &mut values);
                            writer.write_event(Event::PI(BytesPI::new(t))).map_err(io)?;
                        }
                        e => writer.write_event(e).map_err(io)?,
                    }
                }
            }
        }
        let out = String::from_utf8(writer.into_inner()).map_err(|e| e.to_string())?;
        Ok((out, values))
    }

    /// Text pass over a comment, doctype or processing instruction.
    fn scrub_markup(&mut self, raw: &str, values: &mut String) -> String {
        let new = self.scrub(raw);
        values.push_str(&new);
        values.push('\n');
        new
    }

    /// Writes the buffered text of the current element, replaced if a
    /// rule applies, else exactly as read.
    fn flush(
        &mut self,
        writer: &mut Writer<Vec<u8>>,
        key: Option<&str>,
        pieces: &mut Vec<Piece>,
        values: &mut String,
    ) -> Result<(), String> {
        if pieces.is_empty() {
            return Ok(());
        }
        let io = |e: std::io::Error| e.to_string();
        let mut value = String::new();
        let mut cdata = false;
        for piece in pieces.iter() {
            match piece {
                Piece::Text(raw) => value.push_str(&unescape(raw).unwrap_or_else(|_| raw.into())),
                Piece::Ref(name) => {
                    let raw = format!("&{name};");
                    value.push_str(&unescape(&raw).map(|c| c.into_owned()).unwrap_or(raw));
                }
                Piece::CData(text) => {
                    cdata = true;
                    value.push_str(text);
                }
            }
        }
        let new = if value.trim().is_empty() {
            value.clone()
        } else {
            self.value(key, &value, false)
        };
        values.push_str(&new);
        values.push('\n');
        if new == value {
            for piece in pieces.drain(..) {
                let event = match piece {
                    Piece::Text(raw) => Event::Text(BytesText::from_escaped(raw)),
                    Piece::Ref(name) => Event::GeneralRef(BytesRef::new(name)),
                    Piece::CData(text) => Event::CData(BytesCData::new(text)),
                };
                writer.write_event(event).map_err(io)?;
            }
        } else {
            pieces.clear();
            let event = if cdata && !new.contains("]]>") {
                Event::CData(BytesCData::new(new))
            } else {
                Event::Text(BytesText::new(&new).into_owned())
            };
            writer.write_event(event).map_err(io)?;
        }
        Ok(())
    }

    /// Applies the rules to the attribute values of a tag.
    fn attributes<'a>(
        &mut self,
        start: BytesStart<'a>,
        values: &mut String,
    ) -> Result<BytesStart<'a>, String> {
        let mut attrs = Vec::new();
        let mut changed = false;
        for attr in start.attributes().with_checks(false) {
            let attr = attr.map_err(|e| e.to_string())?;
            let key = attr.key.as_ref().to_owned();
            let raw = attr.value.clone().into_owned();
            let value = unescape(&raw).map(|v| v.into_owned()).unwrap_or(raw);
            let new = self.value(Some(&key), &value, false);
            values.push_str(&new);
            values.push('\n');
            changed |= new != value;
            attrs.push((key, new));
        }
        if !changed {
            return Ok(start);
        }
        let name = start.name().as_ref().to_owned();
        let mut new = BytesStart::new(name);
        for (k, v) in &attrs {
            new.push_attribute((k.as_str(), v.as_str()));
        }
        Ok(new)
    }

    /// What in `text` still looks identifying: a known real value, an
    /// IPv4, IPv6 or MAC address outside the replacement ranges. Returns
    /// the kinds found, never the values.
    pub fn leaks(&self, text: &str) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        if let Some(re) = self.known_regex()
            && !self.known_matches(&re, text).is_empty()
        {
            kinds.push("known real value");
        }
        if self
            .patterns
            .ipv4_run
            .find_iter(text)
            .filter_map(|m| ipv4_parts(m.as_str()))
            .any(|o| !ipv4_allowed(o))
        {
            kinds.push("IPv4 address");
        }
        if self
            .ipv6_matches(text)
            .iter()
            .any(|(_, _, a)| !ipv6_allowed(a))
        {
            kinds.push("IPv6 address");
        }
        if self
            .patterns
            .mac
            .find_iter(text)
            .any(|m| !mac_allowed(m.as_str()))
        {
            kinds.push("MAC address");
        }
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml(a: &mut Anonymizer, input: &str) -> String {
        a.anonymize("x.xml", input).0
    }

    #[test]
    fn classify_keys() {
        assert_eq!(classify("serial_no", false), Some(Kind::Serial));
        assert_eq!(classify("SerialNumber", false), Some(Kind::Serial));
        assert_eq!(classify("hostname", false), Some(Kind::Host));
        assert_eq!(classify("serverName", false), Some(Kind::Host));
        assert_eq!(classify("server_name", false), Some(Kind::Host));
        assert_eq!(classify("nas_name", false), Some(Kind::Host));
        assert_eq!(classify("authSid", false), Some(Kind::Secret));
        assert_eq!(classify("sid", false), Some(Kind::Secret));
        assert_eq!(classify("access_token", false), Some(Kind::Secret));
        assert_eq!(classify("pwd", false), Some(Kind::Secret));
        assert_eq!(classify("Password", false), Some(Kind::Secret));
        assert_eq!(classify("qcloud_cuid", false), Some(Kind::Secret));
        assert_eq!(classify("SUID", false), Some(Kind::Secret));
        assert_eq!(classify("jwt", true), Some(Kind::Secret));
        assert_eq!(classify("session_id", true), Some(Kind::Secret));
        assert_eq!(classify("Cookie", true), Some(Kind::Secret));
        // The SNMP community string of the UPS page is a password.
        assert_eq!(classify("community", false), Some(Kind::Secret));
        assert_eq!(classify("snmp_community", false), Some(Kind::Secret));
        assert!(!is_descriptive("community_status"));
        assert_eq!(classify("username", false), Some(Kind::Personal));
        assert_eq!(classify("email", false), Some(Kind::Personal));
        assert_eq!(classify("ddns_host", false), Some(Kind::Personal));
        assert_eq!(classify("domain", false), Some(Kind::Personal));
        assert_eq!(classify("workgroup", false), Some(Kind::Personal));
        assert_eq!(classify("volumeLabel", false), Some(Kind::Volume));
        assert_eq!(classify("pool_name", false), Some(Kind::Volume));
        assert_eq!(classify("shareName", false), Some(Kind::Share));
        assert_eq!(classify("disk_alias", false), Some(Kind::Alias));
        assert_eq!(classify("name", true), Some(Kind::Container));
        assert_eq!(classify("Names", true), Some(Kind::Container));
        assert_eq!(classify("name", false), None);
        assert!(is_descriptive("userType") && is_descriptive("session_timeout"));
        assert!(!is_descriptive("password_status") && !is_descriptive("serial_type"));
        assert_eq!(classify("cpu_usage", false), None);
    }

    #[test]
    fn serials_are_consistent() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><serial_no>Q1</serial_no><serial>Q2X</serial><serialNumber>Q1</serialNumber></r>",
        );
        assert_eq!(
            out,
            "<r><serial_no>SERIAL-A</serial_no><serial>SERIAL-B</serial><serialNumber>SERIAL-A</serialNumber></r>"
        );
        assert_eq!(letters(25), "Z");
        assert_eq!(letters(26), "AA");
    }

    #[test]
    fn cdata_stays_cdata_and_numbers_stay() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<QDocRoot><hostname><![CDATA[Keller-NAS]]></hostname><cpu><![CDATA[12.5]]></cpu><total>1234567</total></QDocRoot>",
        );
        assert_eq!(
            out,
            "<QDocRoot><hostname><![CDATA[nas-demo]]></hostname><cpu><![CDATA[12.5]]></cpu><total>1234567</total></QDocRoot>"
        );
    }

    #[test]
    fn unchanged_text_keeps_escapes() {
        let mut a = Anonymizer::new();
        let input = "<?xml version=\"1.0\"?>\n<r>\n  <model>TS&amp;464</model>\n  <!-- c -->\n</r>";
        assert_eq!(xml(&mut a, input), input);
    }

    #[test]
    fn secrets_and_personal_values() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><authPassed>1</authPassed><authSid>abc123xyz</authSid><username>frank</username><ddns_enable>1</ddns_enable><user_quota>no</user_quota><email>a@b.example</email><workgroup>HOME</workgroup><domain>corp.example</domain></r>",
        );
        assert_eq!(
            out,
            "<r><authPassed>1</authPassed><authSid>REDACTED</authSid><username>REDACTED</username><ddns_enable>1</ddns_enable><user_quota>no</user_quota><email>REDACTED</email><workgroup>REDACTED</workgroup><domain>REDACTED</domain></r>"
        );
    }

    #[test]
    fn volumes_shares_aliases_numbered() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><volumeLabel>Daten</volumeLabel><pool_name>Pool A</pool_name><volumeLabel>Daten</volumeLabel><shareName>Fotos</shareName><disk_alias>Links</disk_alias></r>",
        );
        assert_eq!(
            out,
            "<r><volumeLabel>Volume 1</volumeLabel><pool_name>Volume 2</pool_name><volumeLabel>Volume 1</volumeLabel><shareName>Share 1</shareName><disk_alias>Alias 1</disk_alias></r>"
        );
    }

    #[test]
    fn attributes_are_replaced() {
        let mut a = Anonymizer::new();
        let out = xml(&mut a, r#"<disk serial="WD-123" slot="1" ip="10.1.2.3"/>"#);
        assert_eq!(out, r#"<disk serial="SERIAL-A" slot="1" ip="192.0.2.1"/>"#);
    }

    #[test]
    fn real_values_disappear_everywhere() {
        let mut a = Anonymizer::new();
        let input = "<r><serial>Q19ABC</serial><note>disk of Q19ABC ok</note><hostname>Keller</hostname><title>Keller (TS-464)</title></r>";
        a.learn("x.xml", input);
        let out = xml(&mut a, input);
        assert!(!out.contains("Q19ABC"), "{out}");
        assert!(!out.contains("Keller"), "{out}");
        assert!(out.contains("<note>disk of SERIAL-A ok</note>"), "{out}");
        assert!(out.contains("<title>nas-demo (TS-464)</title>"), "{out}");
    }

    #[test]
    fn learn_makes_late_values_known_early() {
        let mut a = Anonymizer::new();
        let first = "<r><note>Fotos is full</note></r>";
        let second = "<r><shareName>Fotos</shareName></r>";
        a.learn("1.xml", first);
        a.learn("2.xml", second);
        assert_eq!(xml(&mut a, first), "<r><note>Share 1 is full</note></r>");
    }

    #[test]
    fn secrets_are_replaced_even_if_neutral() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><session_id>30</session_id><session_timeout>30</session_timeout><pwd>1</pwd></r>",
        );
        assert_eq!(
            out,
            "<r><session_id>REDACTED</session_id><session_timeout>30</session_timeout><pwd>REDACTED</pwd></r>"
        );
        a.add_secret("1234");
        assert_eq!(a.scrub("pin 1234"), "pin REDACTED");
    }

    #[test]
    fn replacements_are_no_leaks() {
        let mut a = Anonymizer::new();
        let input = "<r><shareName>Plexdaten</shareName><authSid>REDACTED</authSid><note>Plexdaten ok</note></r>";
        let (_, _) = a.anonymize("x.xml", input);
        let (out, _) = a.anonymize(
            "cs.json",
            r#"{"name":"plex","path":"/share/Plexdaten/plex"}"#,
        );
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["name"], "container-1");
        assert_eq!(v["path"], "/share/Share 1/container-1");
        let (out, values) = a.anonymize("x.xml", input);
        assert!(out.contains("<note>Share 1 ok</note>"), "{out}");
        assert!(a.leaks(&values).is_empty(), "{values}");
        assert!(a.leaks("container-1 REDACTED").is_empty());
    }

    #[test]
    fn default_names_stay_in_text() {
        let mut a = Anonymizer::new();
        let input = "<r><volumeLabel>DATA</volumeLabel><shareName>Container</shareName><path>/share/CACHEDEV1_DATA/.qpkg/container-station/data</path></r>";
        let (out, values) = a.anonymize("x.xml", input);
        assert_eq!(
            out,
            "<r><volumeLabel>Volume 1</volumeLabel><shareName>Share 1</shareName><path>/share/CACHEDEV1_DATA/.qpkg/container-station/data</path></r>"
        );
        assert!(a.leaks(&values).is_empty());
    }

    #[test]
    fn descriptive_keys_keep_only_setting_values() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><userType>local</userType><ddns_status>kellernas.myqnapcloud.com</ddns_status><user_count>Frank Meier</user_count></r>",
        );
        assert_eq!(
            out,
            "<r><userType>local</userType><ddns_status>REDACTED</ddns_status><user_count>REDACTED</user_count></r>"
        );
    }

    #[test]
    fn generic_names_take_the_meaning_of_their_parent() {
        let mut a = Anonymizer::new();
        let out = xml(
            &mut a,
            "<r><pool><poolID>1</poolID><name>Fotos</name></pool><poolName>Archiv</poolName><shareList><item><name>Musik</name></item></shareList><func><name>S_MENU_40</name></func></r>",
        );
        assert_eq!(
            out,
            "<r><pool><poolID>1</poolID><name>Volume 1</name></pool><poolName>Volume 2</poolName><shareList><item><name>Share 1</name></item></shareList><func><name>S_MENU_40</name></func></r>"
        );
    }

    #[test]
    fn known_values_need_word_boundaries() {
        let mut a = Anonymizer::new();
        a.add_known("local", REDACTED);
        a.add_known("ab", REDACTED); // too short, ignored
        a.add_known("kellernas", DEMO_HOST);
        assert_eq!(a.scrub("localhost local ab"), "localhost REDACTED ab");
        assert_eq!(
            a.scrub("kellernas_1 kellernas-x /kellernas/ kellernas2"),
            "nas-demo_1 nas-demo-x /nas-demo/ kellernas2"
        );
        assert_eq!(a.leaks("job kellernas_snap"), vec!["known real value"]);
    }

    #[test]
    fn ipv4_rules() {
        let mut a = Anonymizer::new();
        let out = a.scrub(
            "ip 192.168.1.10 gw 192.168.1.1 mask 255.255.255.0 any 0.0.0.0 lo 127.0.0.1 again 192.168.1.10",
        );
        assert_eq!(
            out,
            "ip 192.0.2.1 gw 192.0.2.2 mask 255.255.255.0 any 0.0.0.0 lo 127.0.0.1 again 192.0.2.1"
        );
        // OIDs and versions with more parts are no addresses.
        let oid = "iso.3.6.1.4.1.24681.1.2.1.0 = 5.2.4.3079";
        assert_eq!(a.scrub(oid), oid);
    }

    #[test]
    fn ipv6_rules() {
        let mut a = Anonymizer::new();
        let out =
            a.scrub("v6 fe80::1c2b:3cff:fe4d:5e6f%eth0, lo ::1, dev:2a02:810::42: time 12:30:45");
        assert_eq!(
            out,
            "v6 2001:db8::1%eth0, lo ::1, dev:2001:db8::2: time 12:30:45"
        );
    }

    #[test]
    fn mac_rules() {
        let mut a = Anonymizer::new();
        assert_eq!(a.scrub("SNMPv2-MIB::sysName.0"), "SNMPv2-MIB::sysName.0");
        let out = a.scrub("eth0 24:5E:BE:01:02:03 eth1 24-5e-be-01-02-03 bc ff:ff:ff:ff:ff:ff");
        assert_eq!(
            out,
            "eth0 02:00:00:00:00:01 eth1 02:00:00:00:00:01 bc ff:ff:ff:ff:ff:ff"
        );
    }

    #[test]
    fn snmp_text_blanks_system_lines() {
        let mut a = Anonymizer::new();
        let walk = "SNMPv2-MIB::sysName.0 = STRING: kellernas\nSNMPv2-MIB::sysLocation.0 = STRING: \"Keller\"\niso.3.6.1.4.1.24681.1.2.4.0 = STRING: \"kellernas up\"\n";
        let (out, _) = a.anonymize("snmp.txt", walk);
        assert_eq!(
            out,
            "SNMPv2-MIB::sysName.0 = STRING: REDACTED\nSNMPv2-MIB::sysLocation.0 = STRING: REDACTED\niso.3.6.1.4.1.24681.1.2.4.0 = STRING: \"REDACTED up\"\n"
        );
    }

    #[test]
    fn json_container_station() {
        let mut a = Anonymizer::new();
        let input = r#"{"data":[{"name":"plex","names":["/plex","/media"],"ip":"172.17.0.2","cpu":1.5}],"token":"t0k3n","username":"frank"}"#;
        let (out, values) = a.anonymize("cs_container.json", input);
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["data"][0]["name"], "container-1");
        // `/plex` is the same container as `plex`.
        assert_eq!(v["data"][0]["names"][0], "/container-1");
        assert_eq!(v["data"][0]["names"][1], "/container-2");
        assert_eq!(v["data"][0]["ip"], "192.0.2.1");
        assert_eq!(v["data"][0]["cpu"], 1.5);
        assert_eq!(v["token"], REDACTED);
        assert_eq!(v["username"], REDACTED);
        assert!(values.contains("container-1"));
    }

    #[test]
    fn broken_xml_falls_back_to_text() {
        let mut a = Anonymizer::new();
        let (out, _) = a.anonymize("x.xml", "<html>error from 10.0.0.5<br></html>");
        assert_eq!(out, "<html>error from 192.0.2.1<br></html>");
    }

    #[test]
    fn leaks_finds_left_over_values() {
        let mut a = Anonymizer::new();
        a.add_known("kellernas", DEMO_HOST);
        assert!(
            a.leaks("all fine 192.0.2.4 02:00:00:00:00:01 2001:db8::3 ::1")
                .is_empty()
        );
        assert_eq!(a.leaks("left 10.1.1.1"), vec!["IPv4 address"]);
        assert_eq!(a.leaks("left fe80::1"), vec!["IPv6 address"]);
        assert_eq!(a.leaks("left 24:5e:be:01:02:03"), vec!["MAC address"]);
        assert_eq!(a.leaks("host KellerNAS here"), vec!["known real value"]);
    }

    #[test]
    fn comments_are_scrubbed_and_checked() {
        let mut a = Anonymizer::new();
        let (out, values) = a.anonymize(
            "x.xml",
            "<r><!-- from 10.0.0.9 --><?pi at 10.0.0.9?><x>1</x></r>",
        );
        assert_eq!(
            out,
            "<r><!-- from 192.0.2.1 --><?pi at 192.0.2.1?><x>1</x></r>"
        );
        assert!(values.contains("192.0.2.1"));
        a.add_known("kellernas", DEMO_HOST);
        let (_, values) = a.anonymize("y.xml", "<r><!-- kellernas --></r>");
        assert!(a.leaks(&values).is_empty(), "{values}");
    }

    #[test]
    fn leaks_checks_values_not_markup() {
        let mut a = Anonymizer::new();
        let (out, values) =
            a.anonymize("x.xml", "<r><shareName>data</shareName><data>1</data></r>");
        assert!(out.contains("<data>1</data>"));
        assert!(a.leaks(&values).is_empty(), "{values}");
    }
}
