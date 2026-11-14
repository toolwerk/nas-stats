//! The list of requests sent after the login (`requests.txt`). Every line
//! is checked before the tool logs in, so a typo or a command never
//! reaches the NAS: the tool only reads.

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// File name of the answer without extension, `[a-z0-9_]+`.
    pub name: String,
    /// Path below `/cgi-bin/` with query, without `sid`.
    pub target: String,
}

/// Query keys whose value names the action of a CGI call.
const ACTION_KEYS: &[&str] = &[
    "func", "subfunc", "act", "action", "op", "do", "todo", "cmd", "command",
];

/// Action values starting with one of these change something (or start
/// a job, or write a log entry) and are refused.
const WRITING: &[&str] = &[
    "set", "save", "apply", "add", "del", "remove", "create", "edit", "format", "reboot",
    "shutdown", "restart", "start", "stop", "run", "log", "umount", "unmount", "mount", "eject",
    "update", "upgrade", "enable", "disable", "reset", "scan", "test", "wake", "standby", "kill",
    "wipe", "erase", "clean", "clear", "init", "import", "restore", "repair",
];

/// The same verbs as one part of a value (`hd_delete`, `smart_test`).
/// `update` and `log` are left out here: `firm_update` only reads.
const WRITING_PART: &[&str] = &[
    "set", "save", "apply", "add", "del", "delete", "remove", "create", "edit", "format", "reboot",
    "shutdown", "restart", "start", "stop", "run", "umount", "unmount", "mount", "eject",
    "upgrade", "enable", "disable", "reset", "scan", "test", "wake", "standby", "kill", "wipe",
    "erase", "clean", "clear", "init", "import", "restore", "repair",
];

/// Checks one request. The error says what is wrong.
pub fn check_request(name: &str, target: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!("name `{name}` must be [a-z0-9_]+"));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !path.ends_with(".cgi") {
        return Err(format!("path `{path}` is no .cgi"));
    }
    if path.contains("..") || path.starts_with('/') || path.contains("://") {
        return Err(format!("path `{path}` must stay below /cgi-bin/"));
    }
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = key.to_ascii_lowercase();
        if key == "sid" {
            return Err("`sid=` is added by the tool, remove it".to_owned());
        }
        // `chart_func` and the like name the action as well.
        if ACTION_KEYS.contains(&key.as_str()) || key.ends_with("func") {
            let value = value.to_ascii_lowercase();
            if let Some(word) = WRITING.iter().find(|w| value.starts_with(**w)) {
                return Err(format!(
                    "`{key}={value}` starts with `{word}` and may change the NAS"
                ));
            }
            if let Some(word) = value
                .split(['_', '-', '.'])
                .find(|part| WRITING_PART.contains(part))
            {
                return Err(format!(
                    "`{key}={value}` contains `{word}` and may change the NAS"
                ));
            }
        }
    }
    Ok(())
}

/// Parses `requests.txt`: `<name> <target>` per line, `#` starts a
/// comment, blank lines are skipped. Collects every error with its line
/// number.
pub fn parse_requests(text: &str) -> Result<Vec<Request>, Vec<String>> {
    let mut requests = Vec::new();
    let mut errors = Vec::new();
    let mut names = BTreeSet::new();
    for (i, line) in text.lines().enumerate() {
        let nr = i + 1;
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(target), None) = (parts.next(), parts.next(), parts.next()) else {
            errors.push(format!("line {nr}: expected `<name> <path>?<query>`"));
            continue;
        };
        if let Err(e) = check_request(name, target) {
            errors.push(format!("line {nr}: {e}"));
            continue;
        }
        if !names.insert(name.to_owned()) {
            errors.push(format!("line {nr}: name `{name}` is used twice"));
            continue;
        }
        requests.push(Request {
            name: name.to_owned(),
            target: target.to_owned(),
        });
    }
    if errors.is_empty() {
        Ok(requests)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: &str = include_str!("../requests.txt");

    #[test]
    fn shipped_list_passes() {
        let list = parse_requests(SHIPPED).expect("requests.txt is valid");
        assert!(list.iter().any(|r| r.name == "sysinfo"));
        assert!(list.iter().any(|r| r.name == "qsmart"));
        for r in &list {
            assert!(check_request(&r.name, &r.target).is_ok(), "{}", r.name);
        }
    }

    #[test]
    fn accepts_reading_request() {
        assert!(check_request("sysinfo", "management/manaRequest.cgi?subfunc=sysinfo").is_ok());
        assert!(check_request("x1", "disk/qsmart.cgi").is_ok());
    }

    #[test]
    fn rejects_bad_names() {
        assert!(check_request("Sysinfo", "a.cgi").is_err());
        assert!(check_request("sys-info", "a.cgi").is_err());
        assert!(check_request("", "a.cgi").is_err());
    }

    #[test]
    fn rejects_bad_paths() {
        assert!(check_request("a", "management/manaRequest?x=1").is_err());
        assert!(check_request("a", "../authLogout.cgi").is_err());
        assert!(check_request("a", "/cgi-bin/a.cgi").is_err());
        assert!(check_request("a", "https://evil/a.cgi").is_err());
    }

    #[test]
    fn rejects_sid() {
        assert!(check_request("a", "a.cgi?x=1&sid=abc").is_err());
        assert!(check_request("a", "a.cgi?SID=abc").is_err());
    }

    #[test]
    fn rejects_writing_actions() {
        for (key, value) in [
            ("func", "set_config"),
            ("subfunc", "Reboot"),
            ("act", "delete"),
            ("action", "start_job"),
            ("op", "remove"),
            ("func", "format_disk"),
            ("subfunc", "shutdown"),
            ("act", "save"),
            ("action", "apply"),
            ("op", "add_user"),
            ("func", "create_share"),
            ("func", "edit"),
            ("func", "restart"),
            ("func", "stop"),
            ("func", "run_now"),
            ("func", "log_clear"),
            ("chart_func", "set_x"),
            ("func", "umount_external"),
            ("func", "eject"),
            ("func", "hd_delete"),
            ("func", "smart_test"),
            ("func", "hd_standby"),
            ("func", "update_all"),
            ("do", "reset"),
            ("cmd", "wake"),
        ] {
            let target = format!("a.cgi?{key}={value}");
            assert!(check_request("a", &target).is_err(), "{target}");
        }
        // Only the action keys count, and `firm_update` only reads.
        assert!(check_request("a", "a.cgi?mode=set").is_ok());
        assert!(check_request("a", "a.cgi?subfunc=firm_update").is_ok());
    }

    #[test]
    fn parse_reports_line_numbers_and_duplicates() {
        let text =
            "# c\nsysinfo a.cgi?subfunc=sysinfo\n\nsysinfo b.cgi\nbad\nx c.cgi?func=reboot\n";
        let errors = parse_requests(text).expect_err("invalid list");
        assert_eq!(errors.len(), 3);
        assert!(errors[0].starts_with("line 4:") && errors[0].contains("twice"));
        assert!(errors[1].starts_with("line 5:"));
        assert!(errors[2].starts_with("line 6:"));
    }

    #[test]
    fn parse_skips_comments() {
        let list = parse_requests("a x.cgi?func=get # trailing\n# b y.cgi\n").expect("valid");
        assert_eq!(
            list,
            vec![Request {
                name: "a".into(),
                target: "x.cgi?func=get".into()
            }]
        );
    }
}
