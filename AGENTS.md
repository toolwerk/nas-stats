# NAS Stats - Projektregeln

## Zweck

Statistik- und Zustands-App für NAS verschiedener Hersteller, mit Widgets. QNAP hat Priorität 1 (eigenes Testgerät), Synology, TrueNAS und Unraid folgen. Rust-Core per UniFFI, SwiftUI für iOS und macOS. Windows und Linux später per Tauri.

Recherche: Vault `04 Ressourcen/App-Ideen/App-Idee NAS Stats.md`. Konzept: `NAS Stats Konzept.md` daneben. Vorbilder: Token Stats (`~/dev/token-stats`) und 3D Stats (`~/dev/3d-stats`), dessen Aufbau dieses Repo übernimmt. API-Wissen zu Synology und QNAP steckt schon in NAS Pulse (`~/dev/streamdeck/nas-streamdeck`, `src/lib/synology.ts`, `src/lib/qnap.ts`), zu TrueNAS und Unraid in Homelab Pulse (`~/dev/streamdeck/homelab-streamdeck`).

GitHub: `toolwerk/nas-stats` (privat).

## Setup & Start

- Einmalig: `./setup.sh`
- Rust: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`

## Architektur-Regeln

- Jeder Anbieter ist ein eigenes Crate `provider-<name>` und liefert nur Typen aus `nas-model`. Statistik, FFI und Apps kennen keine Anbieter-Details.
- Fehlt ein Wert beim Anbieter, bleibt er `None`. Nicht raten.
- Jede Parser-Änderung bekommt einen Fixture-Test mit anonymisierter Antwort (Seriennummern, Hostnamen, IPs, Freigabenamen ändern). Nie echte Zugangsdaten oder Gerätetokens einchecken.
- Zeiten sind Unix-Sekunden in UTC, Größen in Bytes, Temperaturen in Grad Celsius.
- Nur lesend zugreifen. Keine Befehle ans NAS (kein Neustart, kein Backup-Start), keine Konfigurationsänderung.
- Abfragen sparsam: höchstens alle 5 Minuten je NAS, letzter Stand bleibt bei Fehlern stehen. Login-Fehler nicht wiederholen, sonst sperrt DSM die IP.
- Demo-Werte sind als solche gekennzeichnet. Für Screenshots nie echte NAS-Daten verwenden.

## Besonderheiten

- Xcode-Projekt nur über `project.yml` und xcodegen, `*.xcodeproj` ist nicht eingecheckt.
- Swift-Targets heißen `NasStats`, Anzeigename „NAS Stats", Bundle-ID `net.staude.nasstats`.
- Git-Identität im Repo: Frank Neumann-Staude, frank@staude.net.
