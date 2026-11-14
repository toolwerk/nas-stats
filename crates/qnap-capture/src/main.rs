//! Records what the own QNAP really answers, so the parsers of
//! `provider-qnap` rest on real fixtures (issue #1). Logs in once, sends
//! every request of `requests.txt` once, only reading, and writes the raw
//! answers to `<out>/<label>/raw` (never commit) and anonymised copies to
//! `<out>/<label>/anon`.
//!
//!   cargo run -p qnap-capture -- --host nas.local
//!       shows the fingerprint of the certificate and stops
//!   cargo run -p qnap-capture -- --host nas.local --pin AB:CD:… [--label monitor]
//!       asks for user and password and records
//!   cargo run -p qnap-capture -- anonymize capture/monitor/raw capture/monitor/anon
//!       anonymises again (after adding files to raw, e.g. an SNMP walk)
//!
//! Only the pinned certificate is trusted, so the password never goes to
//! a device the user did not confirm. Password and session id are never
//! printed nor stored. A failed login is not retried.

mod anon;
mod requests;
mod tls;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use anon::{Anonymizer, DEMO_HOST, REDACTED};
use regex::Regex;
use requests::{Request, parse_requests};

const USAGE: &str = "\
qnap-capture --host HOST[:PORT] [--pin SHA256] [--label NAME] [--out DIR] [--requests FILE]
qnap-capture anonymize RAW_DIR ANON_DIR";

const PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
struct Args {
    host: String,
    pin: Option<String>,
    label: String,
    out: PathBuf,
    requests: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Capture(Args),
    Anonymize { raw: PathBuf, anon: PathBuf },
    Help,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.first().map(String::as_str) == Some("anonymize") {
        return match args {
            [_, raw, anon] => Ok(Command::Anonymize {
                raw: raw.into(),
                anon: anon.into(),
            }),
            _ => Err("anonymize needs RAW_DIR and ANON_DIR".into()),
        };
    }
    let mut host = None;
    let mut parsed = Args {
        host: String::new(),
        pin: None,
        label: "monitor".into(),
        out: "capture".into(),
        requests: "crates/qnap-capture/requests.txt".into(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--host" => host = Some(value()?),
            "--pin" => parsed.pin = Some(value()?),
            "--label" => parsed.label = value()?,
            "--out" => parsed.out = value()?.into(),
            "--requests" => parsed.requests = value()?.into(),
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    parsed.host = host.ok_or("--host is required")?;
    if parsed.label.is_empty()
        || !parsed
            .label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("--label must be [a-z0-9_-]+".into());
    }
    Ok(Command::Capture(parsed))
}

/// Base URL and bare host name from `--host`. Only HTTPS; the port is
/// optional (443).
fn parse_host(host: &str) -> Result<(String, String), String> {
    if host.starts_with("http://") {
        return Err("only HTTPS: drop http://, the tool uses https".into());
    }
    let rest = host.strip_prefix("https://").unwrap_or(host);
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains('/') || rest.contains('@') {
        return Err(format!("--host {host}: expected HOST or HOST:PORT"));
    }
    let name = if let Some(bracketed) = rest.strip_prefix('[') {
        let (addr, tail) = bracketed
            .split_once(']')
            .ok_or(format!("--host {host}: missing ]"))?;
        if !(tail.is_empty() || tail.strip_prefix(':').is_some_and(is_port)) {
            return Err(format!("--host {host}: bad port"));
        }
        addr.to_owned()
    } else if rest.matches(':').count() > 1 {
        return Err(format!("--host {host}: write IPv6 as [addr]:port"));
    } else if let Some((name, port)) = rest.split_once(':') {
        if !is_port(port) {
            return Err(format!("--host {host}: bad port"));
        }
        name.to_owned()
    } else {
        rest.to_owned()
    };
    Ok((format!("https://{rest}"), name))
}

fn is_port(s: &str) -> bool {
    !s.is_empty() && s.len() <= 5 && s.chars().all(|c| c.is_ascii_digit())
}

/// Base64 of the UTF-8 text, what `authLogin.cgi` expects as `pwd`.
fn base64(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.as_bytes().chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Value of the first `<tag>` (plain or CDATA).
fn tag_value(xml: &str, tag: &str) -> Option<String> {
    let t = regex::escape(tag);
    let re = Regex::new(&format!(
        r"(?s)<{t}>\s*(?:<!\[CDATA\[(.*?)\]\]>|([^<]*?))\s*</{t}>"
    ))
    .ok()?;
    let c = re.captures(xml)?;
    c.get(1).or(c.get(2)).map(|m| m.as_str().trim().to_owned())
}

/// One line of `summary.tsv`.
struct Row {
    name: String,
    http: String,
    auth_passed: String,
    bytes: usize,
}

struct Session<'a> {
    client: reqwest::blocking::Client,
    base: String,
    raw: PathBuf,
    rows: Vec<Row>,
    /// Never written to disk: replaced in every raw answer.
    secrets: Vec<String>,
    names: &'a mut Vec<String>,
}

impl Session<'_> {
    /// Stores an answer with secrets removed and adds its summary line.
    fn record(&mut self, file: &str, result: reqwest::Result<(u16, String)>) {
        let name = file.rsplit_once('.').map_or(file, |(n, _)| n).to_owned();
        let (http, body) = match result {
            Ok((status, body)) => (status.to_string(), body),
            Err(e) => {
                // Without the URL: it carries the session id.
                eprintln!("{name}: {}", e.without_url());
                self.rows.push(Row {
                    name,
                    http: "error".into(),
                    auth_passed: "-".into(),
                    bytes: 0,
                });
                return;
            }
        };
        let mut body = body;
        for secret in self.secrets.iter().flat_map(|s| secret_forms(s)) {
            body = body.replace(secret.as_str(), REDACTED);
        }
        let auth_passed = tag_value(&body, "authPassed").unwrap_or_else(|| "-".into());
        if let Err(e) = fs::write(self.raw.join(file), &body) {
            eprintln!("{file}: cannot write: {e}");
        }
        println!(
            "{name}: HTTP {http}, authPassed {auth_passed}, {} bytes",
            body.len()
        );
        self.names.push(file.to_owned());
        self.rows.push(Row {
            name,
            http,
            auth_passed,
            bytes: body.len(),
        });
    }

    fn get(&mut self, file: &str, path_and_query: &str) {
        let result = self
            .client
            .get(format!("{}{path_and_query}", self.base))
            .send();
        self.record(file, read(result));
    }
}

/// A secret as it may show up in an answer: plain, XML-escaped,
/// JSON-escaped, URL-encoded. Shorter than 3 characters is skipped, it
/// would hit everywhere.
fn secret_forms(secret: &str) -> Vec<String> {
    if secret.chars().count() < 3 {
        return Vec::new();
    }
    let json = serde_json::to_string(secret).unwrap_or_default();
    let json = json.trim_matches('"').to_owned();
    let url: String = secret
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let mut forms = vec![
        secret.to_owned(),
        quick_xml::escape::escape(secret).into_owned(),
        json,
        url,
    ];
    forms.sort();
    forms.dedup();
    // Longest first, so a form is not cut by a shorter one.
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms
}

/// Status and body of a response.
fn read(result: reqwest::Result<reqwest::blocking::Response>) -> reqwest::Result<(u16, String)> {
    result.and_then(|r| {
        let status = r.status().as_u16();
        r.text().map(|t| (status, t))
    })
}

fn write_summary(dir: &Path, rows: &[Row]) {
    let mut text = String::from("name\thttp\tauthPassed\tbytes\n");
    for r in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            r.name, r.http, r.auth_passed, r.bytes
        ));
    }
    let path = dir.join("summary.tsv");
    match fs::write(&path, text) {
        Ok(()) => println!("summary: {}", path.display()),
        Err(e) => eprintln!("{}: {e}", path.display()),
    }
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_owned()
}

fn capture(args: &Args) -> ExitCode {
    // The list is checked before anything goes over the network.
    let list = match fs::read_to_string(&args.requests) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{}: {e}", args.requests.display());
            return ExitCode::from(2);
        }
    };
    let list: Vec<Request> = match parse_requests(&list) {
        Ok(list) => list,
        Err(errors) => {
            for e in errors {
                eprintln!("{}: {e}", args.requests.display());
            }
            return ExitCode::from(2);
        }
    };
    let (base, host_name) = match parse_host(&args.host) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let (client, seen) = match tls::client(args.pin.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // Handshake first, without credentials: shows or checks the pin.
    let probe = client.get(format!("{base}/")).send();
    let fingerprint = seen.lock().ok().and_then(|s| s.clone());
    if let Err(e) = probe {
        match (&args.pin, fingerprint) {
            (None, Some(fp)) => {
                println!("Certificate fingerprint (SHA-256) of {host_name}:\n  {fp}");
                println!(
                    "Compare it with the one the browser shows, then run again with --pin {fp}"
                );
                return ExitCode::SUCCESS;
            }
            (Some(pin), Some(fp)) if !tls::same_fingerprint(pin, &fp) => {
                eprintln!("Certificate does not match the pin.\n  pin:  {pin}\n  seen: {fp}");
                return ExitCode::FAILURE;
            }
            _ => {
                eprintln!("{base}: {}", e.without_url());
                return ExitCode::FAILURE;
            }
        }
    }

    let label_dir = args.out.join(&args.label);
    let raw = label_dir.join("raw");
    let anon_dir = label_dir.join("anon");
    if let Err(e) = fs::create_dir_all(&raw) {
        eprintln!("{}: {e}", raw.display());
        return ExitCode::FAILURE;
    }

    let user = ask("User: ");
    let password = match rpassword::prompt_password("Password: ") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("password: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = fs::write(
        raw.join("meta.txt"),
        format!("host={host_name}\nuser={user}\n"),
    ) {
        eprintln!("meta.txt: {e}");
    }

    let encoded = base64(&password);
    let mut names = Vec::new();
    let mut session = Session {
        client,
        base: base.clone(),
        raw: raw.clone(),
        rows: Vec::new(),
        secrets: vec![password.clone(), encoded.clone()],
        names: &mut names,
    };

    // Login, exactly once.
    let login = read(
        session
            .client
            .post(format!("{base}/cgi-bin/authLogin.cgi"))
            .form(&[
                ("user", user.as_str()),
                ("pwd", encoded.as_str()),
                ("serviceKey", "1"),
            ])
            .send(),
    );
    let sid = match &login {
        Ok((_, body)) => tag_value(body, "authSid").filter(|s| !s.is_empty()),
        Err(_) => None,
    };
    let passed = match &login {
        Ok((_, body)) => tag_value(body, "authPassed").as_deref() == Some("1"),
        Err(_) => false,
    };
    if let Some(sid) = &sid {
        session.secrets.push(sid.clone());
    }
    session.record("login.xml", login);
    let Some(sid) = sid.filter(|_| passed) else {
        let need_2sv = fs::read_to_string(raw.join("login.xml"))
            .ok()
            .and_then(|b| tag_value(&b, "need_2sv"))
            .as_deref()
            == Some("1");
        eprintln!(
            "Login failed{}. Not retrying (the NAS may block the address).",
            if need_2sv {
                ": the account needs 2-step verification"
            } else {
                ""
            }
        );
        write_summary(&label_dir, &session.rows);
        let _ = anonymize(&raw, &anon_dir, &session.secrets);
        return ExitCode::FAILURE;
    };

    for r in &list {
        sleep(PAUSE);
        let sep = if r.target.contains('?') { '&' } else { '?' };
        session.get(
            &format!("{}.xml", r.name),
            &format!("/cgi-bin/{}{sep}sid={sid}", r.target),
        );
    }

    sleep(PAUSE);
    session.get(
        "session_expired.xml",
        "/cgi-bin/management/manaRequest.cgi?subfunc=sysinfo&hd=no&multicpu=1&sid=invalid",
    );

    // Container Station has its own login. Tried once per run: a
    // rejection may count as a failed login at the NAS.
    sleep(PAUSE);
    let cs = session
        .client
        .post(format!("{base}/container-station/api/v1/login"))
        .json(&serde_json::json!({ "username": user, "password": password }))
        .send();
    let cs = read(cs);
    let cs_ok = cs.as_ref().is_ok_and(|(status, _)| *status == 200);
    session.record("cs_login.json", cs);
    if cs_ok {
        sleep(PAUSE);
        session.get("cs_container.json", "/container-station/api/v1/container");
    }

    sleep(PAUSE);
    session.get(
        "logout.xml",
        &format!("/cgi-bin/authLogout.cgi?logout=1&sid={sid}"),
    );

    write_summary(&label_dir, &session.rows);
    anonymize(&raw, &anon_dir, &session.secrets)
}

/// Files of `raw` in a stable order: login and sysinfo first, they name
/// the device.
fn raw_files(raw: &Path) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(raw)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n != "meta.txt" && !n.starts_with('.'))
        .collect();
    names.sort_by_key(|n| {
        let rank = match n.as_str() {
            "login.xml" => 0,
            "sysinfo.xml" => 1,
            _ => 2,
        };
        (rank, n.clone())
    });
    Ok(names)
}

/// Anonymises every file of `raw` into `anon`. A file in which
/// [`Anonymizer::leaks`] still finds something is not written; its name
/// and the kind of finding are shown, never the value.
fn anonymize(raw: &Path, anon: &Path, secrets: &[String]) -> ExitCode {
    let names = match raw_files(raw) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{}: {e}", raw.display());
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = fs::create_dir_all(anon) {
        eprintln!("{}: {e}", anon.display());
        return ExitCode::FAILURE;
    }
    let mut a = Anonymizer::new();
    // Password and session id of this run, kept in memory only: a file
    // that still holds one is refused.
    for form in secrets.iter().flat_map(|s| secret_forms(s)) {
        a.add_known(&form, REDACTED);
    }
    if let Ok(meta) = fs::read_to_string(raw.join("meta.txt")) {
        for line in meta.lines() {
            match line.split_once('=') {
                // An IP as host is handled by the address rules.
                Some(("host", host)) if host.parse::<std::net::IpAddr>().is_err() => {
                    a.add_known(host, DEMO_HOST);
                    if let Some((first, _)) = host.split_once('.') {
                        a.add_known(first, DEMO_HOST);
                    }
                }
                Some(("user", user)) => a.add_known(user, REDACTED),
                _ => {}
            }
        }
    }
    let mut files = Vec::new();
    let mut failed = false;
    for name in &names {
        match fs::read_to_string(raw.join(name)) {
            Ok(text) => {
                a.learn(name, &text);
                files.push((name, text));
            }
            Err(e) => {
                eprintln!("{name}: not read ({e}), not written");
                failed = true;
            }
        }
    }
    for (name, text) in &files {
        let target = anon.join(name);
        let (out, values) = a.anonymize(name, text);
        let leaks = a.leaks(&values);
        if leaks.is_empty() {
            if let Err(e) = fs::write(&target, out) {
                eprintln!("{}: {e}", target.display());
                failed = true;
            }
        } else {
            // No stale copy from an earlier run either.
            let _ = fs::remove_file(&target);
            eprintln!("{name}: not written, still contains: {}", leaks.join(", "));
            failed = true;
        }
    }
    println!(
        "anonymised: {} of {} files in {}",
        files.len(),
        names.len(),
        anon.display()
    );
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Anonymize { raw, anon }) => anonymize(&raw, &anon, &[]),
        Ok(Command::Capture(args)) => capture(&args),
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn capture_defaults() {
        let Ok(Command::Capture(a)) = parse_args(&args(&["--host", "nas.local"])) else {
            panic!("capture expected");
        };
        assert_eq!(a.host, "nas.local");
        assert_eq!(a.pin, None);
        assert_eq!(a.label, "monitor");
        assert_eq!(a.out, PathBuf::from("capture"));
        assert_eq!(
            a.requests,
            PathBuf::from("crates/qnap-capture/requests.txt")
        );
    }

    #[test]
    fn capture_options() {
        let Ok(Command::Capture(a)) = parse_args(&args(&[
            "--host",
            "nas",
            "--pin",
            "AB",
            "--label",
            "admin",
            "--out",
            "o",
            "--requests",
            "r.txt",
        ])) else {
            panic!("capture expected");
        };
        assert_eq!(a.pin.as_deref(), Some("AB"));
        assert_eq!(a.label, "admin");
        assert_eq!(a.out, PathBuf::from("o"));
        assert_eq!(a.requests, PathBuf::from("r.txt"));
    }

    #[test]
    fn bad_arguments() {
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["--host"])).is_err());
        assert!(parse_args(&args(&["--host", "n", "--bogus"])).is_err());
        assert!(parse_args(&args(&["--host", "n", "--label", "../x"])).is_err());
        assert!(parse_args(&args(&["anonymize", "raw"])).is_err());
        assert_eq!(
            parse_args(&args(&["anonymize", "r", "a"])),
            Ok(Command::Anonymize {
                raw: "r".into(),
                anon: "a".into()
            })
        );
    }

    #[test]
    fn hosts() {
        assert_eq!(
            parse_host("nas.local"),
            Ok(("https://nas.local".into(), "nas.local".into()))
        );
        assert_eq!(
            parse_host("https://10.0.0.2:8443/"),
            Ok(("https://10.0.0.2:8443".into(), "10.0.0.2".into()))
        );
        assert_eq!(
            parse_host("[fd00::2]:443"),
            Ok(("https://[fd00::2]:443".into(), "fd00::2".into()))
        );
        assert!(parse_host("http://nas.local").is_err());
        assert!(parse_host("fd00::2").is_err());
        assert!(parse_host("nas:80x").is_err());
        assert!(parse_host("nas/x").is_err());
        assert!(parse_host("").is_err());
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(""), "");
        assert_eq!(base64("f"), "Zg==");
        assert_eq!(base64("fo"), "Zm8=");
        assert_eq!(base64("foo"), "Zm9v");
        assert_eq!(base64("foobar"), "Zm9vYmFy");
        assert_eq!(base64("pässwört"), "cMOkc3N3w7ZydA==");
    }

    #[test]
    fn secret_forms_cover_encodings() {
        let forms = secret_forms("a&b\"c d");
        for f in [
            "a&b\"c d",
            "a&amp;b&quot;c d",
            "a&b\\\"c d",
            "a%26b%22c%20d",
        ] {
            assert!(forms.contains(&f.to_owned()), "{f} in {forms:?}");
        }
        assert!(secret_forms("ab").is_empty());
    }

    #[test]
    fn tag_values() {
        let xml = "<QDocRoot><authPassed><![CDATA[1]]></authPassed><authSid>ab12</authSid><x></x></QDocRoot>";
        assert_eq!(tag_value(xml, "authPassed").as_deref(), Some("1"));
        assert_eq!(tag_value(xml, "authSid").as_deref(), Some("ab12"));
        assert_eq!(tag_value(xml, "x").as_deref(), Some(""));
        assert_eq!(tag_value(xml, "need_2sv"), None);
    }

    #[test]
    fn raw_files_order_and_skip_meta() {
        let dir = std::env::temp_dir().join(format!("qnap-capture-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        for n in ["a.xml", "sysinfo.xml", "meta.txt", "login.xml", "b.json"] {
            fs::write(dir.join(n), "x").expect("write");
        }
        let names = raw_files(&dir).expect("list");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(names, vec!["login.xml", "sysinfo.xml", "a.xml", "b.json"]);
    }

    #[test]
    fn anonymize_replaces_run_secrets() {
        let base = std::env::temp_dir().join(format!("qnap-capture-sec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let raw = base.join("raw");
        let anon = base.join("anon");
        fs::create_dir_all(&raw).expect("dir");
        fs::write(raw.join("a.xml"), "<r><x>echo s3cr&amp;t</x></r>").expect("xml");
        let code = anonymize(&raw, &anon, &["s3cr&t".to_owned()]);
        let out = fs::read_to_string(anon.join("a.xml")).unwrap_or_default();
        let _ = fs::remove_dir_all(&base);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(out, "<r><x>echo REDACTED</x></r>");
    }

    #[test]
    fn anonymize_refuses_leaking_file() {
        let base = std::env::temp_dir().join(format!("qnap-capture-anon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let raw = base.join("raw");
        let anon = base.join("anon");
        fs::create_dir_all(&raw).expect("dir");
        fs::write(
            raw.join("meta.txt"),
            "host=kellernas.fritz.box\nuser=frank\n",
        )
        .expect("meta");
        fs::write(
            raw.join("sysinfo.xml"),
            "<r><hostname>kellernas</hostname><ip>10.0.0.2</ip><note>frank</note></r>",
        )
        .expect("xml");
        // The share name hides in a JSON key, which the rules leave alone.
        fs::write(raw.join("cs.json"), r#"{"Fotos": 1, "shareName": "Fotos"}"#).expect("json");
        let code = anonymize(&raw, &anon, &[]);
        let sysinfo = fs::read_to_string(anon.join("sysinfo.xml")).unwrap_or_default();
        let json_written = anon.join("cs.json").exists();
        let _ = fs::remove_dir_all(&base);
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(
            sysinfo,
            "<r><hostname>nas-demo</hostname><ip>192.0.2.1</ip><note>REDACTED</note></r>"
        );
        assert!(!json_written);
    }
}
