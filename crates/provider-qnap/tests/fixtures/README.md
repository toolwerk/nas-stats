# QNAP-Fixtures

Echte Antworten eines eigenen QNAP, aufgezeichnet mit `qnap-capture` (Issue #1) und anonymisiert. Sie sind die Referenz für die Parser von `provider-qnap`, weil die Login-Doku von QNAP offline ist.

## Herkunft

- Aufgezeichnet: 2026-11-14 und 2026-11-15
- Gerät: TS-1685 (`modelName` TS-X85, Plattform `X86_GRANTLEY`), Intel Xeon D-1521, 16 GB RAM, 22 Laufwerke in `qsmart.xml`, USV über USB
- Firmware: QTS 5.2.11, Build 20261023 (nicht QuTS hero, `is_zfs` 0)
- Befehl: `cargo run -p qnap-capture -- --host <nas>:<port> --pin <fingerabdruck> [--label <name>]`, danach `qnap-capture anonymize`
- SNMP: `snmpwalk -v2c -c <community> <nas> .1.3.6.1.4.1.24681.1`

## Anonymisierung

`qnap-capture` ersetzt nach Schlüssel Seriennummern (`SERIAL-A` …), Hostnamen (`nas-demo`), Session, Token, Benutzer, Mail, DDNS, Domain, Arbeitsgruppe, `SUID` und `cuid` (`REDACTED`), Volumes und Pools (`Volume n`), Freigaben (`Share n`), Aliasse (`Alias n`) und Container-Namen (`container-n`), in jedem Text IPv4 (`192.0.2.n`), IPv6 (`2001:db8::n`) und MAC (`02:00:00:00:00:nn`). Struktur, Zahlen und CDATA bleiben. Die Standardnamen von QTS (`DATA`, `Container`, `Public` …) ersetzt das Werkzeug nur unter ihrem Schlüssel, im übrigen Text bleiben sie stehen, damit Pfade wie `CACHEDEV7_DATA` stimmen; deshalb heißt dasselbe Volume in `snmp_nas_mib.txt` noch `Data`. Eine Datei, in der noch ein bekannter echter Wert, eine fremde IP oder MAC steht, schreibt das Werkzeug nicht. In `cs_container.json` bleiben Container-IDs, Image-Digests und die Image-Namen (bis auf den Teil, der dem Containernamen gleicht) unverändert; sie sind zufällig bzw. öffentlich.

## Dateien

| Datei | Abfrage | Konto |
|---|---|---|
| `login.xml` | `POST authLogin.cgi` (`user`, `pwd` Base64, `serviceKey=1`) | admin |
| `sysinfo.xml` | `management/manaRequest.cgi?subfunc=sysinfo&hd=no&multicpu=1` | admin |
| `syshealth.xml` | `management/manaRequest.cgi?subfunc=sysinfo&sysHealth=1` | admin |
| `firmware.xml` | `sys/sysRequest.cgi?subfunc=firm_update` | admin |
| `disk_usage.xml` | `management/chartReq.cgi?chart_func=disk_usage&disk_select=all&include=all` | admin |
| `bandwidth_qsm40.xml` | `management/chartReq.cgi?chart_func=QSM40bandwidth` | admin |
| `bandwidth.xml` | `management/chartReq.cgi?chart_func=bandwidth` | admin |
| `qsmart.xml` | `disk/qsmart.cgi?func=all_hd_data` | admin |
| `external_disks.xml` | `disk/disk_manage.cgi?func=external_get_all` | admin |
| `external_devices.xml` | `devices/devRequest.cgi?func=getExternalDev` | admin |
| `session_expired.xml` | `sysinfo` mit `sid=invalid` | – |
| `logout.xml` | `authLogout.cgi?logout=1` | admin |
| `cs_login.json` | `POST /container-station/api/v1/login` | admin |
| `cs_container.json` | `GET /container-station/api/v1/container` | admin |
| `snmp_nas_mib.txt` | SNMP-Walk über `NAS-MIB` (`.1.3.6.1.4.1.24681.1`) | SNMP v2c |
| `login.user.xml` | wie `login.xml` | normaler Benutzer (Gruppe `everyone`) |
| `sysinfo.user.xml` | wie `sysinfo.xml`; alle anderen CGI-Abfragen liefern für dieses Konto dieselbe Antwort | normaler Benutzer |
| `cs_login.user.json` | wie `cs_login.json` | normaler Benutzer |
| `cs_container.user.json` | wie `cs_container.json`, HTTP 401 | normaler Benutzer |
| `login_wrong_password.xml` | `POST authLogin.cgi` mit falschem Passwort, **nachgebaut** (#4) | – |
| `login_2sv.xml` | `POST authLogin.cgi` bei aktiver 2-Stufen-Verifizierung, **nachgebaut** (#4) | – |
| `sysinfo_empty.xml` | wie `sysinfo.xml` mit leeren Feldern, **nachgebaut** (#5) | – |
| `syshealth_good.xml` | wie `syshealth.xml` mit Zustand `good`, **nachgebaut** (#5) | – |
| `firmware_available.xml` | wie `firmware.xml` mit angebotenem Update, **nachgebaut** (#5) | – |
| `disk_usage_warning.xml` | wie `disk_usage.xml` mit einem Volume `DataVol1` bei 87,1 % belegt, **nachgebaut** (#6) | – |
| `qsmart_warning.xml` | wie `qsmart.xml` mit vier Platten (Platte 3 bei 52 °C mit SMART `Warning`, Platte 4 mit unbekanntem Text) und einem leeren Steckplatz, **nachgebaut** (#6) | – |
| `bandwidth_items.xml` | wie `bandwidth.xml` mit Schnittstellen als `item` (`id`, `dname` oder `name`) statt `eth<n>`, für den Rückfall auf `QSM40bandwidth`, **nachgebaut** (#7) | – |
| `hbs_jobs.json` | `GET /cc3/v1/users/system/jobs` (Kopfzeile `X-QNAP-SID`), ein Backup-Auftrag mit Status `Finished`, **nachgebaut** aus den Feldern, die am 2026-11-17 in den DevTools zu sehen waren (#41) | – |
| `hbs_jobs_mixed.json` | wie `hbs_jobs.json` mit Status `Running`, einem abgeschalteten Auftrag, einem Restore-Auftrag, einem Auftrag ohne `statistics` und `enable` und einem ohne `_id`, `has_more` true, **nachgebaut** (#41) | – |

## Antworten

- Jede CGI-Antwort hat `QDocRoot` mit `authPassed`; die meisten Werte stehen in CDATA. Viele Antworten (z.B. `external_devices.xml`) wiederholen den Block `model`/`firmware`/`hostname` aus `sysinfo`.
- Ein Konto ohne Admin-Rechte meldet sich an (`authPassed` 1, `isAdmin` 0), bekommt aber auf jede Abfrage nur `authPassed` 0 ohne Daten. Die Container Station antwortet `forbidden: not admin user`.
- `login_wrong_password.xml` und `login_2sv.xml` sind nicht aufgezeichnet (ein Fehlversuch zählt bei QTS für die Sperre), sondern in der Form der anderen Antworten nachgebaut, nach NAS Pulse und der früheren Login-Doku (`errorValue`, `need_2sv`).
- Eine ungültige Session antwortet mit HTTP 200 und `authPassed` 0, nicht mit 401.
- `QSM40bandwidth` liefert auf QTS 5.2.11 nur `authPassed` 1 ohne Werte; die Bandbreite kommt aus `bandwidth`.
- In `bandwidth.xml` sind `rx` und `tx` Bytes je Messintervall, nicht je Sekunde; das Intervall steht in `bandwidth_interval` (5 s). Schnittstellen stehen als `eth<n>` in `bandwidth_info`, gelistet in `eth_index_list` und `wlan_index_list`; `link_running` 0 heißt kein Kabel. Die Form mit `item`-Kindern (`bandwidth_items.xml`) ist nicht aufgezeichnet, sondern nach dem Muster älterer Firmware nachgebaut.
- Die Container Station hat eine eigene Anmeldung (`POST /container-station/api/v1/login`, JSON mit `username` und `password` im Klartext) und hält die Session per Cookie. `cs_login.json` meldet `isAdmin` true, `cs_login.user.json` `isAdmin` false. Wie sie antwortet, wenn sie nicht installiert ist, ist nicht aufgezeichnet; `provider-qnap` nimmt HTTP 404 oder eine Antwort, die kein JSON ist, als „nicht installiert“, bei der Anmeldung auch eine Weiterleitung (3xx), und eine Anmelde-Antwort mit Fehlerobjekt ohne `isAdmin` (`code`, wie in `cs_container.user.json`) als abgelehnt.
- `syshealth.xml` meldet `warning`, `firmware.xml` `newVersion` `none`.
- Lüfter stehen in `sysinfo.xml`: Systemlüfter als `sysfan_count`, `sysfan<n>` (U/min) und `sysfan_fail<n>`, CPU-Lüfter als `cpufan_count`, `cpufan<n>` und `fan_fail<n>`. `sysfan_alias` ist ein einzelnes Feld ohne Nummer und lässt sich keinem Lüfter zuordnen; die Lüfter heißen deshalb „Lüfter <n>“ und „CPU-Lüfter <n>“. Eine angeschlossene USV nennt nur `external_devices.xml` (Gerätetyp `ups`), ohne Zustand, Ladung oder Restlaufzeit.
- `disk_usage.xml` hat je Volume nur einen Zahlencode in `volumeStatus` (`0` und `44`; QTS zeigt `44` bei einem zu gut 94 % vollen Volume als Warning); Größen stehen getrennt in `volumeUseList`, verbunden über `volumeValue`. `qsmart.xml` hat keine Betriebsstunden, leere Steckplätze haben kein `Model`.
- `disk_manage.cgi?store=…` (Speicherpools, `lvList`, `volumeInfo` u.a.) antwortet auf GET nur mit `unknown cmd format.` (aufgezeichnet 2026-11-15); die Oberfläche sendet POST. Die Abfragen und `sysmonitor` stehen in `crates/qnap-capture/requests.txt`, sind aber noch nicht aufgezeichnet.
- Hybrid Backup Sync 3 hat kein CGI, sondern eine REST-Schnittstelle des Pakets auf dem Port von QTS: `GET /cc3/v1/users/system/jobs` mit der QTS-Session in der Kopfzeile `X-QNAP-SID` und `Content-Type: application/json`, Antwort `{ "jobs": [...], "total": n, "has_more": bool }`. Je Auftrag `_id`, `_type`, `job_type`, `enable`, `name`, `schedule` (Cron in der Zeitzone des NAS) und `statistics` (`status`, `progress`, `start_time` und `stop_time` als ISO 8601 mit Zeitzone, Bytes). Beobachtet ist nur der Status `Finished` (TS-1685, HBS 3 v26.4.6); jeden anderen Wert nimmt `provider-qnap` als unbekannt. Die echte Antwort ist nicht aufgezeichnet, beide Dateien sind nachgebaut. `qpkg/HybridBackup/cgi/api_server.cgi` zeigt, ob der Dienst läuft (502 oder 503: läuft nicht); `provider-qnap` fragt es nicht ab, eine fehlende oder abgelehnte Liste bleibt einfach leer.
