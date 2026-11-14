# NAS Stats

Statistik und Zustand Ihrer NAS auf iPhone, iPad und Mac. Die App liest Speicherbelegung, Platten mit Temperatur und SMART, CPU, RAM, Netzwerk, Backup-Aufgaben und USV aus und zeigt den Verlauf über Wochen und Monate. Dazu kommen Widgets für den Zustand und den Füllstand. Vorbild sind [Token Stats](https://github.com/mahype/token-stats) und 3D Stats.

Status: Projektstart. Die Recherche zu Herstellern und Schnittstellen läuft, das Konzept folgt. Noch kein Anbieter angebunden.

## Anbieter (geplant)

| Anbieter | Schnittstelle | Stand |
|---|---|---|
| Synology DSM 7 | Web API (`/webapi/entry.cgi`) mit Session und Gerätetoken | geplant |
| QNAP QTS / QuTS hero | CGI-Endpunkte der Weboberfläche (XML) | geplant |
| TrueNAS SCALE / Community Edition | JSON-RPC über WebSocket mit API-Key | geplant |

Weitere Anbieter folgen nach der Recherche. Die App greift nur lesend zu.

## Aufbau

```
SwiftUI (iOS, macOS, Widgets) ──UniFFI── Rust-Core
```

- `crates/nas-model`: gemeinsames Datenmodell für NAS, Volume, Platte, Backup-Aufgabe und Messwert
- `crates/provider-<name>`: je Anbieter ein Crate, das API-Antworten in das Datenmodell übersetzt (folgt)
- `crates/stats-core`, `crates/stats-ffi`, `apps/NasStats`: Statistik, Swift-Hülle und App (folgen)

Windows und Linux folgen später über Tauri aus demselben Rust-Core.

## Voraussetzungen

- Rust (stable)
- Für die Apps später Xcode 27 und xcodegen (`brew install xcodegen`)

## Setup

```bash
./setup.sh
```

Das Skript setzt die Git-Identität, baut und testet den Rust-Core.

## Bauen und testen

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Lizenz

MIT oder Apache-2.0, nach Wahl.
