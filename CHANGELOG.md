# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `nas-http` crate: HTTPS layer for all providers with certificate pinning after the first contact (SHA-256 fingerprint), `CertificateChanged` with old and new fingerprint, 20 s time limit, no retries, no redirects, HTTPS only.
- `ProviderError::Unreachable` carries a detail for the log (#3).
- Project skeleton: Rust workspace with the `nas-model` crate, CI, setup script.
- `nas-model`: shared types for NAS, system status, volumes, disks, backups, UPS, fans, containers, alerts, interfaces, snapshots and samples, the `Provider` trait with `ProviderError`, and salted hashing of disk serials (#2).
- `provider-qnap`: login over `authLogin.cgi` with the session kept in memory, exactly one new login when the session is gone, `PermissionDenied` when the endpoint stays refused, a hard stop after a rejected login or required 2-step verification until the user logs in again or changes the credentials, and logout (#4).
- `qnap-capture`: records the answers of an own QNAP read-only (pinned certificate, one login, request list checked for commands) and anonymises them for the fixtures (#1).
