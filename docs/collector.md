# Sammel-Container

Die Apps fragen ein NAS nur ab, solange sie laufen. Ohne laufenden Mac hat der Verlauf deshalb Lücken. Der Sammel-Container läuft in Docker auf dem NAS selbst, fragt rund um die Uhr ab und speichert den Verlauf. Die Apps holen sich die fehlenden Werte im Heimnetz von ihm (Koppeln und Abgleich in der App: #124).

Was der Container tut:

- Er fragt jedes eingetragene NAS höchstens alle 5 Minuten ab, mit demselben Rust-Core wie die Apps, und speichert Messwerte und Backup-Läufe in `history.sqlite`. Stunden- und Tageswerte, Aufbewahrungsfristen und Prognose sind dieselben wie in der App.
- Er greift nur lesend zu: keine Befehle ans NAS, keine Änderung der Konfiguration.
- Nach einer abgelehnten Anmeldung meldet er sich nicht erneut an, auch nicht nach einem Neustart, bis der Zugang in der Konfiguration geändert ist. Sonst sperrt das NAS womöglich die IP-Adresse.
- Er gibt Messwerte und Backup-Läufe über eine Schnittstelle im Heimnetz heraus (HTTPS, Port 8743, mit Schlüssel). Ereignisse und Mitteilungen bleiben bei der App.
- Derzeit nur QNAP.

Die Apps fragen das NAS weiter selbst ab; der Container ergänzt nur den Verlauf. Das NAS bekommt so eine Abfrage mehr je 5 Minuten.

## Image bauen (Mac)

Voraussetzung: Docker Desktop mit buildx.

```bash
scripts/build-collector-image.sh amd64
```

Für ein QNAP mit Intel- oder AMD-Prozessor `amd64` (Standard), für eines mit ARM-Prozessor `arm64`. Am Ende steht der Pfad der Datei, z. B. `target/collector/nas-collector-0.1.0-amd64.tar`. Eine andere Architektur als die des Macs baut Docker in Emulation; das dauert einige Minuten. Das Image heißt `nas-collector:<version>` und `nas-collector:latest`.

## In Container Station importieren

1. Container Station öffnen, **Images** › **Importieren**, die `.tar`-Datei wählen.
2. Einen Ordner für die Konfiguration anlegen, z. B. `Container/nas-collector` (auf dem NAS `/share/Container/nas-collector`).
3. Dort `collector.toml` ablegen (Vorlage: `docker/collector/collector.example.toml`, siehe unten).
4. **Anwendungen** › **Erstellen**, den Inhalt von `docker/collector/compose.yaml` einfügen und den Pfad zu `collector.toml` anpassen, falls der Ordner anders heißt. Erstellen startet den Container.
5. Im Protokoll des Containers steht nach dem Start `gestartet: 1 NAS, alle 5 Minuten, …` und bald danach `keller: ok`.

## Eigenes Konto

Der Container braucht eigene Zugangsdaten. Auf dem QNAP liefern die nötigen Abfragen derzeit nur mit einem Konto der Gruppe administrators alle Werte (Befund aus #1). Empfehlung: ein eigenes Admin-Konto nur für den Container anlegen, mit eigenem Passwort und ohne 2-Stufen-Verifizierung. So lässt sich der Zugang später sperren, ohne das eigene Konto zu ändern, und eine Sperre wegen falscher Anmeldung trifft nur dieses Konto.

Das Passwort steht im Klartext in `collector.toml` auf dem NAS. Die Datei deshalb nur für den Container lesbar machen: Der Container läuft als Benutzer mit der UID 10001. Per SSH:

```bash
chown 10001 /share/Container/nas-collector/collector.toml
chmod 600 /share/Container/nas-collector/collector.toml
```

Wer Zugriff auf diesen Ordner oder auf Docker auf dem NAS hat, kann das Passwort lesen.

## Konfiguration

`collector.toml` (Vorlage `docker/collector/collector.example.toml`):

| Feld | Bedeutung |
|---|---|
| `listen` | Adresse der Schnittstelle, Standard `0.0.0.0:8743` |
| `interval_minutes` | Abstand der Abfragen je NAS, Standard und Minimum 5 |
| `[[nas]]` | ein Abschnitt je NAS |
| `id` | eigene Kennung, nur Buchstaben, Ziffern, `.`, `_`, `-`; jede nur einmal |
| `provider` | derzeit nur `qnap` |
| `base_url` | Adresse des NAS, nur `https://`, vom Container aus gesehen, also mit der IP-Adresse im Heimnetz |
| `fingerprint` | SHA-256-Fingerabdruck des Zertifikats des NAS (`AB:CD:…`) |
| `user`, `password` | das eigene Konto (oben) |

Den Fingerabdruck gibt der Container selbst aus, ohne Anfrage ans NAS:

```bash
docker run --rm nas-collector:latest fingerprint https://192.168.1.20:443
```

Er muss zu dem passen, den die Weboberfläche des NAS bzw. die App beim Hinzufügen zeigt. Eine fehlerhafte Konfiguration beendet den Container mit Exit 1; die Meldung nennt das Feld, nie das Passwort. Nach einer Änderung den Container neu starten.

## Mit der App koppeln

Fingerabdruck des Containers und Schlüssel für die App:

```bash
docker exec nas-collector nas-collector pairing
```

Beim ersten Start legt der Container dafür ein selbstsigniertes Zertifikat (`tls/`) und einen zufälligen Schlüssel (`token`) im Datenordner an, nur für ihn lesbar. Die App vertraut genau diesem Zertifikat und schickt den Schlüssel bei jeder Anfrage mit. Wer neu koppeln will, löscht `tls/` oder `token` im Datenordner und startet den Container neu; danach muss jede App neu gekoppelt werden.

Prüfen von einem Rechner im Heimnetz:

```bash
curl -k -H "Authorization: Bearer <schlüssel>" https://<nas>:8743/v1/nas
```

Ohne Schlüssel antwortet die Schnittstelle mit 401.

## Port nicht freigeben

Port 8743 ist nur für das Heimnetz gedacht. Im Router keine Portweiterleitung dafür einrichten und den Port nicht über myQNAPcloud oder UPnP freigeben. Unterwegs gleicht die App ab, sobald sie wieder im Heimnetz ist (oder über VPN).

## Sperre aufheben

Lehnt das NAS die Anmeldung ab (falsches Passwort, 2-Stufen-Verifizierung, geändertes Zertifikat), steht im Protokoll `… angehalten, bis sich der Zugang in der Konfiguration ändert`, und der Container fragt dieses NAS nicht mehr ab. Aufheben:

1. In `collector.toml` Benutzer, Passwort, Adresse oder Fingerabdruck korrigieren.
2. Den Container neu starten.

Ein geänderter Zugang hebt die Sperre auf. War der Zugang richtig und die Ursache auf dem NAS (z. B. 2-Stufen-Verifizierung inzwischen abgeschaltet), den gespeicherten Stand löschen und neu starten:

```bash
docker exec nas-collector rm /data/poll-state.json
docker restart nas-collector
```

## Datenordner

Der Datenordner `/data` liegt im Volume `nas-collector-data`:

| Datei | Inhalt |
|---|---|
| `history.sqlite` | Verlauf aller NAS (Rohwerte 7 Tage, Stundenwerte 90 Tage, Tageswerte und Backup-Läufe ohne Frist) |
| `poll-state.json` | je NAS Zeit der letzten Abfrage, Sperre und ein Hash des Zugangs |
| `tls/cert.der`, `tls/key.der` | Zertifikat der Schnittstelle |
| `token` | Schlüssel für die Apps |

## Schnittstelle

Jede Anfrage braucht `Authorization: Bearer <schlüssel>`, sonst 401.

- `GET /v1/nas`: die eingetragenen NAS mit `id`, `provider`, `name`, `model`, `base_url`, `fingerprint`, `last_seen`, `blocked` und `error`.
- `GET /v1/export?nas=<id>&since=<unix-sekunden>`: der Verlauf eines NAS seit `since` (ohne `since` alles): Rohwerte, Stunden- und Tageswerte, Backup-Läufe, dazu `until` als Zeitpunkt für die nächste Abfrage. Unbekanntes NAS 404, `since` keine Zahl 400.

Andere Pfade 404, andere Methoden als GET 405.
