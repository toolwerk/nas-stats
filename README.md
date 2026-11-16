# NAS Stats

Statistik und Zustand Ihrer NAS auf iPhone, iPad und Mac. Die App liest Speicherbelegung, Platten mit Temperatur und SMART, CPU, RAM, Netzwerk, Backup-Aufgaben und USV aus und zeigt den Verlauf über Wochen und Monate. Dazu kommen Widgets für den Zustand und den Füllstand (Widget „NAS“ in Klein mit Füllstandsring und Mittel mit Balken je Volume und Kennzahlen, Widget „Füllstand“ in Klein mit Balken je Volume, Widget „Alle NAS“ in Groß mit einer Zeile je NAS und dem Verlauf der letzten 30 Tage, dazu auf dem iPhone-Sperrbildschirm das Widget „NAS“ rund, rechteckig und als Zeile). Vorbild sind [Token Stats](https://github.com/mahype/token-stats) und 3D Stats.

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

- `crates/nas-model`: gemeinsames Datenmodell für NAS, Volume, Platte, Backup-Aufgabe und Messwert, dazu die Befunde je Snapshot mit Schwellen (`Thresholds`) und die Ereignisse (`detect_events`, `detect_unreachable`): Ein neuer Snapshot wird mit den offenen Ereignissen verglichen und meldet geöffnete, gestiegene und erledigte, mit Hysterese (Platte 3 °C, Volume 1 Prozentpunkt unter der Schwelle) und „nicht erreichbar“ nach 15 Minuten ohne Stand. Das Speichern übernimmt `stats-core` (`write_events`, `open_events`)
- `crates/nas-demo`: deterministischer Demo-Datensatz für Screenshots, Store-Bilder und den Demo-Modus: die drei NAS des Mockups (Keller, Büro, Homelab) mit aktuellem Stand, 90 Tagen Verlauf und Backup-Läufen sowie den Ereignissen des Mockups (Platte 3 und DataVol1 offen, USV auf Batterie und Homelab nicht erreichbar erledigt). Alle Zeiten hängen am übergebenen Zeitpunkt, jede Kennung beginnt mit `demo-`, jede Adresse endet auf `.demo.invalid`
- `crates/nas-http`: HTTPS-Schicht für alle Anbieter. Beim ersten Kontakt liest sie nur den SHA-256-Fingerabdruck des Zertifikats, danach akzeptiert sie genau dieses Zertifikat (auch selbstsigniert oder abgelaufen). Nur HTTPS, 20 Sekunden Zeitlimit je Anfrage, keine Wiederholungen, Cookies je NAS nur im Speicher
- `crates/qnap-capture`: Werkzeug, das echte Antworten des eigenen QNAP nur lesend aufzeichnet und für die Fixtures anonymisiert (`cargo run -p qnap-capture -- --help`, Rohdaten unter `capture/`, nie einchecken)
- `crates/provider-qnap`: QNAP QTS über die CGI-Endpunkte. Anmeldung über `authLogin.cgi` mit Session im Speicher, genau eine Neuanmeldung nach Ablauf, nach einer abgelehnten Anmeldung (falsches Passwort, 2-Stufen-Verifizierung) keine weitere bis zur Nutzeraktion; Abmelden beim Entfernen des NAS. Systemwerte, Volumes und Platten mit SMART (`QnapClient::storage`), Durchsatz je Netzwerkanschluss (`chartReq.cgi?chart_func=bandwidth`) und Container der Container Station (eigene Anmeldung, leer ohne Container Station oder ohne Admin-Recht). `QnapProvider` setzt Systemwerte, Speicher, Netz und Container nacheinander zu einem Snapshot zusammen (Modell, Firmware, CPU, RAM, Temperaturen, Lüfter, Warnungen, Volumes, Platten, Netzanschlüsse, Container). Ein HTTP-Fehlerstatus bei Speicher, Netz oder Container lässt nur diesen Teil leer, alle anderen Fehler beenden den Abruf; Pools und Backups bleiben leer
- `crates/provider-<name>`: je Anbieter ein Crate, das API-Antworten in das Datenmodell übersetzt (weitere folgen)
- `crates/stats-ffi`: Swift-Hülle des Cores über UniFFI. Records für alles, was in einem Snapshot steckt, dazu `NasConfig`, `NasProblem` (Fehlerzustand je NAS) und `NasState`. `NasCore` nimmt die NAS der App an (`addNas`, `removeNas`, `updateCredentials`), fragt sie mit `refresh` ab und liefert mit `states` den Stand aller NAS; `testConnection` fragt ein NAS einmal ab, ohne es anzumelden (ein anschließendes `addNas` übernimmt diesen Snapshot); `savedState` gibt den letzten Snapshot eines NAS als JSON zum Speichern, `restoreState` gibt ihn nach einem Neustart zurück (ein vorhandener Stand bleibt, Fehler, Sperre und Abfragezeit bleiben unberührt). Jeder Snapshot bringt seine Befunde mit (`findings`: Meldungen, Platten ab der Temperaturschwelle oder mit SMART-Fehler, Volumes ab der Warn- bzw. kritischen Füllstandsschwelle oder gestört, Lüfter, USV auf Batterie, Backups fehlgeschlagen oder älter als zwei Tage, Snapshot-Aufgaben und Paritätsprüfung nie überfällig), mit Stufe und deutschem Text, berechnet in `nas-model` (`Snapshot::findings`), dazu die Volumes, deren Füllstand zählt (`capacityVolumeIds`, ein Pool und das Volume darin zählen einmal). Die Schwellen setzt die App mit `setThresholds` (bis dahin 50 °C, 80 und 90 %); der Core rechnet die Befunde aus dem gelesenen Snapshot bei jedem `states` und `refresh` neu, eine geänderte Schwelle wirkt also sofort und ohne Abfrage. Freie Aufrufe: `fetchFingerprint` (Zertifikat vor dem Anlegen), `standardThresholds` (die Schwellen des Cores), `demoSnapshot` (voll befüllte, als Demo markierte Werte), `demoSnapshots` (Keller, Büro und Homelab aus `nas-demo`, als Demo markiert, mit den übergebenen Schwellen, für den Demo-Modus), `coreVersion`. Die App speichert NAS-Liste und Zugang (Keychain) und meldet jedes NAS beim Start an; den letzten Snapshot speichert die App über `savedState`, Zeit der letzten Abfrage und Sperre gibt sie mit `restorePoll` zurück. Jede Abfrage läuft über den Planer aus `stats-core`, auch `refresh` (höchstens eine Abfrage je NAS in 5 Minuten, eine laufende Abfrage wird nicht abgewartet) und `refreshNow` (manuell, frühestens 30 Sekunden nach der letzten). Mit `startPlanner` fragt der Core selbst ab (Abstand `setInterval`) und ruft nach jeder fertigen Abfrage, auch einer fehlgeschlagenen, den `StateListener` der App auf; `stopPlanner` hält ihn an. Jede Abfrage landet im Verlaufsarchiv (`NasCore()` im Speicher, `NasCore.withHistory(path:)` in einer Datei), mit Ereignissen nach den Schwellen aus `setThresholds` und „nicht erreichbar“, wenn das NAS 15 Minuten nach dem letzten Stand nicht antwortet (eine abgelehnte Anmeldung zählt nicht); `removeNas` löscht Verlauf und Ereignisse des NAS. Lesen: `history` (je Metrik und Zeitraum Tag, Woche, Monat, Jahr), `events` (je Zeitraum, für ein oder alle NAS), `backupRuns`, `forecast` (je Volume). `NasCore.demo(now:)` meldet die drei Demo-NAS an, samt Verlauf, Backup-Läufen und Ereignissen aus `nas-demo`. Regeln: nach abgelehnter Anmeldung, zweitem Faktor oder geändertem Zertifikat keine Abfrage mehr bis zum neuen Zugang, der letzte Stand bleibt bei Fehlern stehen. Alle Netzaufrufe laufen auf einer tokio-Runtime des Cores. `crates/uniffi-bindgen` erzeugt die Swift-Bindings
- `scripts/build-ffi.sh`: baut `stats-ffi` als `StatsFFI.xcframework` samt Swift-Bindings nach `apps/NasStats/Generated/` (nicht eingecheckt)
- `apps/NasStats`: Xcode-Projekt aus `project.yml` (xcodegen). Targets `NasStats` (iOS), `NasStatsMac` (Menüleisten-App ohne Dock-Symbol), Widget-Erweiterungen `NasStatsWidgets` (iOS) und `NasStatsWidgetsMac`, gemeinsamer Code in `Shared`. `NasStats/Store` hält den Stand aller NAS für alle Ansichten (`NasStore`): Er fragt die NAS aus `NasAccounts` (Schlüsselbund, Anmeldung am Core beim Start) im eingestellten Abstand ab, frühestens alle 5 Minuten; Sperre und Zeitpunkt der letzten Abfrage merkt er sich über den Neustart hinaus (`queryLog` in der App Group), damit ein Neustart weder sofort erneut abfragt noch eine abgelehnte Anmeldung wiederholt. Den letzten Stand jedes NAS speichert er ebenfalls über den Neustart hinaus (`last-states.json` in der App Group) und zeigt ihn sofort nach dem Start, auch unterwegs ohne Verbindung; ein unlesbarer Stand wird verworfen. Gleichzeitige Abfragerunden fasst er zu einer zusammen. Auf dem iPhone fragt er beim Zurückkehren in die App fällige NAS sofort ab und meldet beim Verlassen eine gelegentliche Hintergrundabfrage an (`BGAppRefreshTask` `net.staude.nasstats.refresh`, ohne Zusage zur Häufigkeit). Die Testabfrage beim Hinzufügen zählt als erste Abfrage. Er sortiert kritisch, Warnung, ok, unbekannt (ein Fehler hebt mindestens auf Warnung, eine Sperre oder eine vom Core abgelehnte Anmeldung auf kritisch), nimmt neue NAS, Entfernen und neuen Zugang (Benutzer und Passwort) an (lehnt der Core ihn ab, bleibt der alte gespeichert), zeigt im Demo-Modus die drei Demo-NAS ohne Abfrage und ruft nach jedem neuen Stand `onUpdate` auf, die Stelle für den Widget-Snapshot (#23). Die Einstellungen (`NasStats/Settings`, iPhone als Tab, Mac als Fenster aus dem Rechtsklick-Menü der Menüleiste, ⌘,) benennen ein NAS um, ändern den Zugang, starten ein angehaltenes NAS mit dem gespeicherten Zugang genau einmal neu, vertrauen nach Prüfung einem neuen Zertifikat und entfernen ein NAS; dazu Abfrageintervall (5, 10, 15 oder 30 Minuten je NAS, `queryInterval`), Schwellen für Füllstand und Plattentemperatur (`thresholds`, färben Ampel und Reihenfolge), Demo-Modus und Datenschutz-Hinweis. Ein Schalter „Zugänge über iCloud-Schlüsselbund teilen“ (`sharesAccessViaICloud`, je Gerät, standardmäßig an) legt die Zugänge aller NAS in den iCloud-Schlüsselbund oder holt sie zurück; Ausschalten fragt vorher. Der Store gibt die Schwellen beim Start und nach jeder Änderung an den Core (auch für die Demo-Daten); der Zustand eines NAS folgt den Befunden des Cores, die App übernimmt sie unverändert und rechnet nichts nach. Nur die Farbe der Füllstandsbalken und der Plattentemperatur bestimmt sie selbst, mit denselben Schwellen. Erster Tab des iPhones ist die Übersicht (`NasStats/Overview`): Kopfsatz mit dem Zustand aller NAS und dem ältesten Stand, eine Karte je NAS mit Befundzeilen, Füllstandsbalken und Kennzahlen, Ziehen zum Aktualisieren (nur NAS, deren Abfrage fällig ist), bei fehlender Verbindung oder Zeitüberschreitung „Nicht erreichbar“ mit dem Zeitpunkt des letzten Stands (in Grau, solange der letzte Stand nicht kritisch war) (die Einstellungen zeigen die genaue Meldung), Tippen öffnet die Detailseite des NAS (`NasStats/Detail`, #18): Kopf, Speicher und Pools, Platten, Backups, System mit USV, Netz und Container aus dem letzten Stand. App Group `group.net.staude.nasstats` (iOS) bzw. `7N2W4T6UPV.net.staude.nasstats` (macOS), Keychain-Gruppe `7N2W4T6UPV.net.staude.nasstats`
- `crates/stats-core`: Verlaufsarchiv in SQLite (`HistoryStore`, `rusqlite` mit eingebautem SQLite). Aus jedem Snapshot schreibt es Belegung und Größe je Volume, Temperatur je Platte, CPU, RAM, Systemtemperatur und Durchsatz je Netzanschluss, dazu den letzten Lauf jeder Backup-Aufgabe. Bei jedem Schreiben verdichtet es die berührten Stunden (Minimum, Mittel, Maximum) und Tage (ein Tag, dessen Stundenwerte schon gelöscht sind, behält seinen Tageswert) und löscht alte Werte, ohne eigenen Zeitgeber: Rohwerte nach 7 Tagen, Stundenwerte nach 90 Tagen, Tageswerte, Backup-Läufe und Ereignisse nie. Stunden und Tage zählen in UTC. Abfrage je Metrik und Zeitraum: 24 Stunden mit jedem Messwert, 7 und 30 Tage mit Stundenwerten, 1 Jahr mit Tageswerten. Ereignisse (#10) speichert es mit Beginn und Ende (`write_events`) und lädt die offenen je NAS für die Erkennung nach einem Neustart (`open_events`). Schema-Version 1 mit schrittweiser Migration (`PRAGMA user_version`). Lesen je Zeitraum auch für Ereignisse aller NAS (`events`). Dazu die Prognose „voll in X Tagen“ (#9, `forecast`, `HistoryStore::forecast`): eine Ausgleichsgerade über die Tageswerte der Belegung der letzten 30 Tage, ab 7 Tageswerten und bei steigender Gerade, sonst kein Trend (`None`), auch wenn das Volume erst in mehr als 10 Jahren voll wäre. Die Tage zählen vom heutigen Punkt der Geraden, nicht vom letzten Wert, damit eine große Löschung die Prognose nicht springen lässt; liegt die Gerade schon über der Größe, sind es 0 Tage. Ergebnis: Tage bis voll, Datum und Steigung in Byte je Tag. Dazu der Abfrage-Planer: fragt jedes NAS höchstens alle 5 Minuten ab, hält nach abgelehnter Anmeldung an und erlaubt manuelles Aktualisieren nach 30 Sekunden

Windows und Linux folgen später über Tauri aus demselben Rust-Core.

Ein klickbares Mockup der Übersicht, des NAS-Details, des Verlaufs mit Prognose, der Widgets und der Mac-Menüleiste liegt unter `docs/mockup/nas-stats-mockup.html` (Demo-Werte).

## Voraussetzungen

- Rust (stable)
- Für die Apps Xcode 27 und xcodegen (`brew install xcodegen`)

## Setup

```bash
./setup.sh
```

Das Skript setzt die Git-Identität, baut und testet den Rust-Core. Unter macOS installiert es dazu die Rust-Targets für Mac und iOS, baut `StatsFFI.xcframework` und erzeugt mit xcodegen `apps/NasStats/NasStats.xcodeproj` (nicht eingecheckt). Danach das Projekt in Xcode öffnen und das Schema `NasStats` (iOS) oder `NasStatsMac` starten.

Nach Änderungen an `project.yml` erneut `xcodegen generate` in `apps/NasStats` ausführen. Den Rust-Core baut Xcode bei jedem Build über das Target `StatsFFI` mit.

## Bauen und testen

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Die Swift-Tests der Bindings und des `NasStore` (`apps/NasStats/Tests`) laufen im Schema `NasStatsMac`:

```bash
scripts/build-ffi.sh debug macosx
cd apps/NasStats && xcodegen generate
xcodebuild test -project NasStats.xcodeproj -scheme NasStatsMac -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO
```

## Lizenz

MIT oder Apache-2.0, nach Wahl.
