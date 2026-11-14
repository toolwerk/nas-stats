# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `nas-http` crate: HTTPS layer for all providers with certificate pinning after the first contact (SHA-256 fingerprint), `CertificateChanged` with old and new fingerprint, 20 s time limit, no retries, no redirects, HTTPS only.
- `NasError` in `nas-model`: `Unreachable`, `Timeout`, `LoginFailed`, `PermissionDenied`, `CertificateChanged`, `Parse`, `Http`.
- Project skeleton: Rust workspace with the `nas-model` crate, CI, setup script.
