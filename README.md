# NAS Stats

Statistik und Zustand Ihrer NAS auf iPhone, iPad und Mac. Die App liest Speicherbelegung, Platten mit Temperatur und SMART, CPU, RAM, Netzwerk, Backup-Aufgaben und USV aus und zeigt den Verlauf über Wochen und Monate. Dazu kommen Widgets für den Zustand und den Füllstand. Vorbild sind [Token Stats](https://github.com/mahype/token-stats) und 3D Stats.

Status: Projektstart. Die Recherche zu Herstellern und Schnittstellen ist abgeschlossen, das Konzept 0.1 steht. QNAP kommt zuerst. Noch kein Anbieter angebunden. Der Umsetzungsplan steht in den Issues, Übersicht in [#47](https://github.com/toolwerk/nas-stats/issues/47).

## Anbieter (geplant)

| Anbieter | Schnittstelle | Stand |
|---|---|---|
| QNAP QTS / QuTS hero | CGI-Endpunkte der Weboberfläche (XML), optional SNMP | V1, Priorität |
| Synology DSM 7 | Web API (`/webapi/entry.cgi`) mit Session und Gerätetoken, optional SNMP | V1.1 |
| TrueNAS SCALE / Community Edition | JSON-RPC 2.0 über `wss://<host>/api/current` mit API-Key (nur über TLS) | V1.1 |
| Unraid 7.2+ | GraphQL mit API-Key | V1.1 |
| Proxmox VE / Backup Server | REST mit API-Token, RRD-Verlauf | später |
| Asustor, TerraMaster, WD, Eigenbau | SNMP (HOST-RESOURCES, UCD-SNMP, IF-MIB, Hersteller-MIBs) | später |
| UGREEN, OpenMediaVault, UniFi UNAS | inoffizielle Schnittstellen | später |

Kein Hersteller liefert den Verlauf der Speicherbelegung per API. NAS Stats speichert deshalb eigene Zeitreihen. Die App greift nur lesend zu.

## Aufbau

```
SwiftUI (iOS, macOS, Widgets) ──UniFFI── Rust-Core
```

- `crates/nas-model`: gemeinsames Datenmodell für NAS, Volume, Platte, Backup-Aufgabe und Messwert
- `crates/nas-http`: HTTPS-Schicht für alle Anbieter. Beim ersten Kontakt liest sie nur den SHA-256-Fingerabdruck des Zertifikats, danach akzeptiert sie genau dieses Zertifikat (auch selbstsigniert oder abgelaufen). Nur HTTPS, 20 Sekunden Zeitlimit je Anfrage, keine Wiederholungen
- `crates/qnap-capture`: Werkzeug, das echte Antworten des eigenen QNAP nur lesend aufzeichnet und für die Fixtures anonymisiert (`cargo run -p qnap-capture -- --help`, Rohdaten unter `capture/`, nie einchecken)
- `crates/provider-qnap`: QNAP QTS über die CGI-Endpunkte. Anmeldung über `authLogin.cgi` mit Session im Speicher, genau eine Neuanmeldung nach Ablauf, nach einer abgelehnten Anmeldung (falsches Passwort, 2-Stufen-Verifizierung) keine weitere bis zur Nutzeraktion; Abmelden beim Entfernen des NAS
- `crates/provider-<name>`: je Anbieter ein Crate, das API-Antworten in das Datenmodell übersetzt (weitere folgen)
- `crates/stats-core`, `crates/stats-ffi`, `apps/NasStats`: Statistik, Swift-Hülle und App (folgen)

Windows und Linux folgen später über Tauri aus demselben Rust-Core.

Ein klickbares Mockup der Übersicht, des NAS-Details, des Verlaufs mit Prognose, der Widgets und der Mac-Menüleiste liegt unter `docs/mockup/nas-stats-mockup.html` (Demo-Werte).

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
