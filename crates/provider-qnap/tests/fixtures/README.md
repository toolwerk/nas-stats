# QNAP-Fixtures

Echte Antworten eines eigenen QNAP, aufgezeichnet mit `qnap-capture` (Issue #1) und anonymisiert. Sie sind die Referenz für die Parser von `provider-qnap`, weil die Login-Doku von QNAP offline ist.

## Herkunft

- Aufgezeichnet: 2026-11-14 und 2026-11-15
- Gerät: TS-1685 (`modelName` TS-X85, Plattform `X86_GRANTLEY`), Intel Xeon D-1521, 16 GB RAM, 22 Laufwerke in `qsmart.xml`, USV über USB
- Firmware: QTS 5.2.11, Build 20261023 (nicht QuTS hero, `is_zfs` 0)
- Befehl: `cargo run -p qnap-capture -- --host <nas>:<port> --pin <fingerabdruck> [--label <name>]`, danach `qnap-capture anonymize`
- SNMP: `snmpwalk -v2c -c <community> <nas> .1.3.6.1.4.1.24681.1`

## Anonymisierung

`qnap-capture` ersetzt nach Schlüssel Seriennummern (`SERIAL-A` …), Hostnamen (`nas-demo`), Session, Token, Benutzer, Mail, DDNS, Domain, Arbeitsgruppe, `SUID` und `cuid` (`REDACTED`), Volumes und Pools (`Volume n`), Freigaben (`Share n`), Aliasse (`Alias n`) und Container-Namen (`container-n`), in jedem Text IPv4 (`192.0.2.n`), IPv6 (`2001:db8::n`) und MAC (`02:00:00:00:00:nn`). Struktur, Zahlen und CDATA bleiben. Die Standardnamen von QTS (`DATA`, `Container`, `Public` …) bleiben in Pfaden wie `CACHEDEV7_DATA` stehen. Eine Datei, in der noch ein echter Wert steht, schreibt das Werkzeug nicht. Die Container-IDs und Image-Digests in `cs_container.json` sind unverändert; sie sind zufällig bzw. öffentlich.

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

## Antworten

- Jede CGI-Antwort hat `QDocRoot` mit `authPassed`; die meisten Werte stehen in CDATA. Viele Antworten (z.B. `external_devices.xml`) wiederholen den Block `model`/`firmware`/`hostname` aus `sysinfo`.
- Ein Konto ohne Admin-Rechte meldet sich an (`authPassed` 1, `isAdmin` 0), bekommt aber auf jede Abfrage nur `authPassed` 0 ohne Daten. Die Container Station antwortet `forbidden: not admin user`.
- Eine ungültige Session antwortet mit HTTP 200 und `authPassed` 0, nicht mit 401.
- `QSM40bandwidth` liefert auf QTS 5.2.11 nur `authPassed` 1 ohne Werte; die Bandbreite kommt aus `bandwidth`.
- `syshealth.xml` meldet `warning`, `firmware.xml` `newVersion` `none`.
- Die Abfragen für Speicherpools und Volumes (`disk_manage.cgi?store=…`) und `sysmonitor` stehen in `crates/qnap-capture/requests.txt`, sind aber noch nicht aufgezeichnet.
