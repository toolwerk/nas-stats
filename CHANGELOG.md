# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Project skeleton: Rust workspace with the `nas-model` crate, CI, setup script.
- `nas-model`: shared types for NAS, system status, volumes, disks, backups, UPS, fans, containers, alerts, interfaces, snapshots and samples, the `Provider` trait with `ProviderError`, and salted hashing of disk serials (#2).
