# Sammel-Container

Die Apps fragen ein NAS nur ab, solange sie laufen. Ohne laufenden Mac hat der Verlauf deshalb Lücken. Der Sammel-Container läuft in Docker auf dem NAS selbst, fragt rund um die Uhr ab und speichert den Verlauf. Die Apps holen sich die fehlenden Werte im Heimnetz von ihm (Koppeln und Abgleich in der App: #124).

Was der Container tut:

- Er fragt jedes eingetragene NAS höchstens alle 5 Minuten ab, mit demselben Rust-Core wie die Apps, und speichert Messwerte und Backup-Läufe in `history.sqlite`. Stunden- und Tageswerte und Aufbewahrungsfristen sind dieselben wie in der App; die Prognose rechnet die App aus dem übernommenen Verlauf.
- Er greift nur lesend zu: keine Befehle ans NAS, keine Änderung der Konfiguration.
- Nach einer abgelehnten Anmeldung meldet er sich nicht erneut an, auch nicht nach einem Neustart, bis der Zugang in der Konfiguration geändert ist. Sonst sperrt das NAS womöglich die IP-Adresse.
- Er gibt Messwerte und Backup-Läufe über eine Schnittstelle im Heimnetz heraus (HTTPS, Port 8743, mit Schlüssel). Ereignisse und Mitteilungen bleiben bei der App.
- Anbieter: QNAP (Benutzer und Passwort) und Unraid 7.2 oder neuer (API-Key).

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
5. Die `compose.yaml` gibt dem Container beim Stoppen 45 Sekunden (`stop_grace_period`), damit eine laufende Abfrage noch gespeichert wird; eine abgelehnte Anmeldung bleibt so als Sperre erhalten. Von Hand mit `docker stop -t 45 nas-collector` stoppen.
6. Im Protokoll des Containers steht nach dem Start `gestartet: 1 NAS, alle 5 Minuten, …` und bald danach `keller: ok`.

## Eigenes Konto

Der Container braucht eigene Zugangsdaten. Auf dem QNAP liefern die nötigen Abfragen derzeit nur mit einem Konto der Gruppe administrators alle Werte (Befund aus #1). Empfehlung: ein eigenes Admin-Konto nur für den Container anlegen, mit eigenem Passwort und ohne 2-Stufen-Verifizierung. So lässt sich der Zugang später sperren, ohne das eigene Konto zu ändern, und eine Sperre wegen falscher Anmeldung trifft nur dieses Konto.

### Unraid

Ein Unraid fragt der Container über die GraphQL-API mit einem API-Key ab, ohne Anmeldung und ohne Session. Den Key unter **Settings** › **Management Access** › **API Keys** anlegen, mit der Rolle **Viewer** (nur lesen; laut Quellcode der Unraid-API reicht sie für alle Abfragen), und in `collector.toml` als `api_key` eintragen; `user` und `password` entfallen.

Der Container spricht nur HTTPS. Unraid liefert ab Werk nur HTTP: unter **Settings** › **Management Access** **Use SSL/TLS** auf **Yes** stellen (selbstsigniertes Zertifikat, Zugriff über die IP-Adresse; empfohlen). Mit **Strict** (Let's-Encrypt-Zertifikat über `myunraid.net`) leitet Unraid Zugriffe über die IP-Adresse auf die `myunraid.net`-Adresse um, der Container folgt keiner Umleitung: Dann muss `base_url` die `myunraid.net`-Adresse aus **Management Access** sein. Bei **Strict** erneuert Unraid das Zertifikat außerdem alle zwei bis drei Monate; danach meldet der Container ein geändertes Zertifikat und hält an, bis der neue Fingerabdruck in `collector.toml` steht.

Platten liest der Container nur aus den Array-Daten, damit schlafende Platten schlafen bleiben; SMART und Betriebsstunden fehlen deshalb. Ein abgelehnter Key hält die Abfrage an wie eine abgelehnte Anmeldung.

### Zugangsdaten schützen

Passwort bzw. API-Key stehen im Klartext in `collector.toml` auf dem NAS. Die Datei deshalb nur für den Container lesbar machen: Der Container läuft als Benutzer mit der UID 10001. Per SSH:

```bash
chown 10001 /share/Container/nas-collector/collector.toml
chmod 600 /share/Container/nas-collector/collector.toml
```

Wer Zugriff auf diesen Ordner oder auf Docker auf dem NAS hat, kann Passwort bzw. API-Key lesen.

## Konfiguration

`collector.toml` (Vorlage `docker/collector/collector.example.toml`):

| Feld | Bedeutung |
|---|---|
| `listen` | Adresse der Schnittstelle, Standard `0.0.0.0:8743` |
| `interval_minutes` | Abstand der Abfragen je NAS, Standard und Minimum 5 |
| `[[nas]]` | ein Abschnitt je NAS |
| `id` | eigene Kennung, nur Buchstaben, Ziffern, `.`, `_`, `-`; jede nur einmal |
| `provider` | `qnap` oder `unraid` |
| `base_url` | Adresse des NAS, nur `https://`, vom Container aus gesehen, also mit der IP-Adresse im Heimnetz |
| `fingerprint` | SHA-256-Fingerabdruck des Zertifikats des NAS (`AB:CD:…`) |
| `user`, `password` | nur QNAP: das eigene Konto (oben) |
| `api_key` | nur Unraid: API-Key der Rolle Viewer (oben) |

Den Fingerabdruck gibt der Container selbst aus, ohne Anfrage ans NAS:

```bash
docker run --rm nas-collector:latest fingerprint https://192.168.1.20:443
```

Er muss zu dem passen, den die Weboberfläche des NAS bzw. die App beim Hinzufügen zeigt. Eine fehlerhafte Konfiguration beendet den Container mit Exit 1; die Meldung nennt das Feld, nie Passwort oder API-Key. Nach einer Änderung den Container neu starten.

## Mit der App koppeln

Fingerabdruck des Containers und Schlüssel für die App:

```bash
docker exec nas-collector nas-collector pairing
```

Beim ersten Start legt der Container dafür ein selbstsigniertes Zertifikat (`tls/`) und einen zufälligen Schlüssel (`token`) im Datenordner an, nur für ihn lesbar. Die App vertraut genau diesem Zertifikat und schickt den Schlüssel bei jeder Anfrage mit. Wer neu koppeln will, löscht `tls/` oder `token` im Datenordner und startet den Container neu; danach muss die App neu gekoppelt werden (Einstellungen › Sammel-Container › Neu koppeln). Mit „Zugänge über iCloud-Schlüsselbund teilen“ genügt das auf einem Gerät, die anderen übernehmen die neue Kopplung.

Prüfen von einem Rechner im Heimnetz:

```bash
curl -k -H "Authorization: Bearer <schlüssel>" https://<nas>:8743/v1/nas
```

Ohne Schlüssel antwortet die Schnittstelle mit 401.

## Port nicht freigeben

Port 8743 ist nur für das Heimnetz gedacht. Im Router keine Portweiterleitung dafür einrichten und den Port nicht über myQNAPcloud oder UPnP freigeben. Unterwegs gleicht die App ab, sobald sie wieder im Heimnetz ist (oder über VPN).

## Sperre aufheben

Lehnt das NAS die Anmeldung ab (falsches Passwort, abgelehnter API-Key, 2-Stufen-Verifizierung, geändertes Zertifikat), steht im Protokoll `… angehalten, bis sich der Zugang in der Konfiguration ändert`, und der Container fragt dieses NAS nicht mehr ab. Aufheben:

1. In `collector.toml` Benutzer, Passwort, API-Key, Adresse oder Fingerabdruck korrigieren.
2. Den Container neu starten.

Ein geänderter Zugang hebt die Sperre auf. War der Zugang richtig und die Ursache auf dem NAS (z. B. 2-Stufen-Verifizierung inzwischen abgeschaltet), den gespeicherten Stand bei gestopptem Container löschen; im laufenden Container schriebe die nächste Abfrage eines anderen NAS ihn sofort neu:

```bash
docker stop -t 45 nas-collector
docker run --rm --volumes-from nas-collector --entrypoint rm nas-collector:latest /data/poll-state.json
docker start nas-collector
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
