# Synology-Fixtures

Die Antworten stammen **nicht vom eigenen Gerät**: Ein Synology-NAS ist nicht vorhanden. Sie sind nach dem „DSM Login Web API Guide“ von Synology und den Antworten nachgebaut, die NAS Pulse (`~/dev/streamdeck/nas-streamdeck`, `src/lib/synology.ts`) verarbeitet. Sitzung und Gerätetoken heißen `REDACTED` bzw. `REDACTED-DID`. Sobald ein echtes Gerät zur Verfügung steht, werden sie durch aufgezeichnete und anonymisierte Antworten ersetzt.

| Datei | Abfrage | Inhalt |
|---|---|---|
| `api_info.json` | `GET /webapi/entry.cgi?api=SYNO.API.Info&version=1&method=query&query=…` | Katalog mit `SYNO.API.Auth` (Version 7) und `SYNO.Core.System` (für die Tests von `call`) |
| `login.json` | `POST /webapi/entry.cgi` (`api=SYNO.API.Auth`, `method=login`) | Erfolg ohne Gerätetoken |
| `login_device_token.json` | dasselbe mit `otp_code` und `enable_device_token=yes` | Erfolg mit Gerätetoken (`did`) |
| `login_2fa.json` | Login ohne Code bei aktiver 2-Faktor-Anmeldung | Fehler 403 |
| `login_code_rejected.json` | Login mit falschem Code | Fehler 404 |
| `login_wrong_password.json` | Login mit falschem Passwort | Fehler 400 |
| `login_ip_blocked.json` | Login, nachdem DSM die IP gesperrt hat | Fehler 407 |
| `session_expired.json` | Abfrage mit abgelaufener Sitzung | Fehler 119 |
| `no_permission.json` | Abfrage mit einem Konto ohne Admin-Rechte | Fehler 105 |
| `data.json` | `SYNO.Core.System` `info` | Erfolg mit Systemwerten (gekürzt) |
| `logout.json` | `GET /webapi/entry.cgi` (`method=logout`) | Erfolg |
