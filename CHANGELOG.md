# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `nas-demo` crate: deterministic demo data as in the mockup (Keller with QNAP TS-464, Büro with Synology DS923+, Homelab with TrueNAS CE 25.10), with 90 days of daily history from a fixed seed and daily backup runs. All times follow the given `now`; ids start with `demo-`, base URLs end in `.demo.invalid`. The DataVol1 history yields "full in 143 days" from a regression over the last 30 days (#22).
- `nas-http` crate: HTTPS layer for all providers with certificate pinning after the first contact (SHA-256 fingerprint), `CertificateChanged` with old and new fingerprint, 20 s time limit, no retries, no redirects, HTTPS only.
- `ProviderError::Unreachable` carries a detail for the log (#3).
- Project skeleton: Rust workspace with the `nas-model` crate, CI, setup script.
- `nas-model`: shared types for NAS, system status, volumes, disks, backups, UPS, fans, containers, alerts, interfaces, snapshots and samples, the `Provider` trait with `ProviderError`, and salted hashing of disk serials (#2).
- `provider-qnap`: login over `authLogin.cgi` with the session kept in memory, exactly one new login when the session is gone, `PermissionDenied` when the endpoint stays refused, a hard stop after a rejected login or required 2-step verification until the user logs in again or changes the credentials, and logout (#4).
- `provider-qnap`: system values from `sysinfo` (model, firmware with build, hostname, uptime, CPU, memory, system and CPU temperature, system and CPU fans), an alert when the system health is not `good`, and an offered firmware update, fetched at most once per hour. A part the account may not read stays empty. `SystemStatus` gains `memory_total_bytes` and `cpu_temperature_c` (#5).
- `provider-qnap`: throughput per network interface in bytes per second from `chart_func=bandwidth` (fallback `QSM40bandwidth`), interfaces without a link left out, and the containers of the Container Station with their own login and session cookie. A missing Container Station, an account without admin rights or a rejected login give an empty list without repeated logins. `nas-http` keeps cookies per NAS and can post JSON (#7).
- Xcode project via xcodegen (`apps/NasStats/project.yml`): iOS app `NasStats`, macOS menu bar app `NasStatsMac` without Dock icon, widget extensions per platform, App Group and keychain group. `stats-ffi` with the core version as first UniFFI call, `uniffi-bindgen`, `scripts/build-ffi.sh` for the XCFramework, `setup.sh` generates the project, CI job `Apps` builds both apps on the Xcode 27 runner (#13).
- `provider-qnap`: `QnapProvider` implements `Provider` and builds a snapshot from the system values; volumes, disks and the network follow with #52 (#12).
- `stats-ffi`: records for every type inside a snapshot (with `used_percent`, `worst_health` and a `demo` flag), `NasConfig`, `NasProblem` and `NasState`; `NasCore` to add, remove, query and re-credential NAS in memory, at most one query per NAS in 5 minutes, no query after a rejected login, second factor or changed certificate until new credentials, the last snapshot kept on errors; `fetch_fingerprint`, `demo_snapshot` and one tokio runtime for all network calls. Swift test `NasStatsMacTests` decodes every record and runs in the `Apps` CI job (#12).
- `qnap-capture`: records the answers of an own QNAP read-only (pinned certificate, one login, request list checked for commands) and anonymises them for the fixtures (#1).
