# Unraid-Fixtures

Antworten der GraphQL-API von Unraid für die Tests von `provider-unraid`.

## Herkunft

- **Von Hand gebaut, nicht von einem Gerät aufgezeichnet.** Es gab beim Bau (Issue #35) kein Unraid zum Testen. Grundlage sind das Schema und der Quellcode der Unraid-API in Version 4.25.0, die mit Unraid 7.2.0 ausgeliefert wird (`unraid/api`, `api/generated-schema.graphql` und die Resolver unter `api/src/unraid-api/graph/resolvers/`), dazu Homelab Pulse (`~/dev/streamdeck/homelab-streamdeck/src/lib/unraid.ts`).
- Abweichungen einer echten Antwort zeigen sich erst am ersten Gerät. Dann die Antwort aufzeichnen, anonymisieren und die betroffene Datei ersetzen.
- Anonymisiert bzw. erfunden: Server-Präfix der IDs (`SERVERID`), Plattenkennungen aus Modell und Seriennummer (`MODEL_SERIAL-A` …), Hostname (`tower-demo`), Freigaben (`share-1` …), Container (`container-1` …), Hersteller und Modell des Boards.

## Dateien

| Datei | Abfrage | Inhalt |
|---|---|---|
| `main.json` | `vars`, `info`, `array` (Pflichtabfrage) | gestartetes Array, letzte Paritätsprüfung abgeschlossen, Parität, vier Datenplätze (einer leer, einer schlafend mit Lesefehlern, einer deaktiviert), Cache ohne Seriennummer in der ID, Kapazität als Text (`BigInt`) |
| `main_minimal.json` | wie `main.json` | fehlende und `null`-Felder, gestopptes Array in Kleinschreibung, Zahl und Flag als Text, Platte ohne Namen, kaputter Listeneintrag, Bootzeit in der Zukunft, Paritätsprüfung `NEVER_RUN` |
| `main_parity_running.json` | wie `main.json` | laufende Paritätsprüfung (12 %), keine Paritätsplatte, Kapazität als Zahl |
| `metrics.json` | `metrics { cpu memory }` | CPU und RAM |
| `shares.json` | `shares` | Freigaben in Kilobyte, eine ohne Namen, eine ohne freien Platz |
| `notifications.json` | `notifications.list(filter: UNREAD)` | `INFO`, `WARNING`, `alert` ohne Betreff und Zeit, ein leerer Eintrag |
| `ups.json` | `upsDevices` | USV im Netzbetrieb (`OL CHRG`), zweites Gerät wird ignoriert |
| `ups_none.json` | `upsDevices` | keine USV |
| `docker.json` | `docker.containers` | laufender, beendeter und pausierter Container (ohne Namen) |
| `docker_disabled.json` | `docker.containers` | Docker-Dienst aus: GraphQL-Fehler ohne `data` |
| `unauthenticated.json` | jede | abgelehnter API-Key (`UNAUTHENTICATED`) |
| `forbidden.json` | jede | fehlendes Recht (`FORBIDDEN`) |
| `parity_history.json` | `parityHistory` | abgeschlossene, abgebrochene und Prüfungen mit Fehlern, Dauer als Text, ein Eintrag ohne Datum |
| `parity_history_empty.json` | `parityHistory` | leerer Verlauf |
| `parity_history_missing.json` | `parityHistory` | GraphQL-Fehler ohne `data` (kein Paritätslog) |

## Einheiten

- Kapazität von Array und Freigaben in Kilobyte zu 1000 Byte, nicht 1024.
- `metrics.memory.total` in Byte, `estimatedRuntime` in Sekunden, `duration` in Sekunden, `progress` und `chargeLevel` in Prozent.
- `info.os.uptime` ist die Bootzeit (ISO 8601), keine Dauer.
- `parityCheckStatus.date` ist der Start der aktuellen bzw. letzten Prüfung (`sbSynced`), `duration` die Zeit bis zum Ende (`sbSynced2`) bzw. bis jetzt; `parityHistory[].date` ist das Ende einer vergangenen Prüfung (Zeitpunkt des Eintrags im Paritätslog). Fehlt das Paritätslog, antwortet `parityHistory` mit einem GraphQL-Fehler.

## Rolle `VIEWER`

Befund aus dem Quellcode der Unraid-API 4.25.0, am Gerät noch zu bestätigen:

- Die genutzten Abfragen verlangen nur Lesen (`AuthAction.READ_ANY`) auf ihrer Ressource, z. B. `ARRAY` für `array` und `parityHistory`, `INFO` für `metrics` (`@UsePermissions` in den Resolvern unter `api/src/unraid-api/graph/resolvers/`).
- Die Rolle `VIEWER` bekommt in `api/src/unraid-api/auth/casbin/policy.ts` `READ_ANY` auf allen Ressourcen außer `API_KEY`. Damit reicht sie für jede Abfrage dieses Anbieters: `vars`, `info`, `array` (samt `parityCheckStatus`), `metrics`, `shares`, `notifications`, `upsDevices`, `docker.containers` und `parityHistory`.
- Alle genutzten Felder stehen im Schema 4.25.0 (`api/generated-schema.graphql`, Tag `v4.25.0`), abgeglichen am 2026-11-17.
- Fehlt ein Recht doch, antwortet die API mit `FORBIDDEN`; der Anbieter lässt dann nur den betroffenen Teil leer (außer bei der Pflichtabfrage).

## Nicht genutzt

- `disks` (SMART-Status, Seriennummer, Betriebsstunden): Die Abfrage weckt schlafende Array-Platten (unraid/api#2018). Platten kommen nur aus `array`.
- `docker.organizer`: nicht nötig und in neueren Versionen umgebaut.
