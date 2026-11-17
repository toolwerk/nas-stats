# TrueNAS-Fixtures

Ohne eigenes TrueNAS stammen die Referenzwerte für die Anmeldung aus der offiziellen Client-Bibliothek von TrueNAS, nicht von einem Gerät.

## `scram_vectors.json`

SCRAM-SHA-512-Nachrichten für einen API-Key, ohne und mit Kanalbindung (`tls-server-end-point`). Der Unit-Test in `src/scram.rs` erzeugt mit denselben Eingaben Client-First und Client-Final und prüft die Signatur des Servers.

- Quelle: [`truenas/api_client`](https://github.com/truenas/api_client), Commit `ecc1f7a`, reine Python-Umsetzung `truenas_api_client/py_scram` (nur Standardbibliothek).
- Erzeugt: 2026-11-17 mit Python 3.14 und einem Skript, das `py_scram` mit festen Werten aufruft. Es ersetzt den Nonce-Generator der Client-First-Nachricht durch einen festen Wert, baut die Server-First-Nachricht aus festem Server-Nonce und Salt, lässt `ClientFinalMessage` den Beweis rechnen und prüft ihn mit `verify_client_final_message` der Bibliothek. Die Signatur des Servers ist `scram_hmac_sha512(ServerKey, AuthMessage)` der Bibliothek, mit der AuthMessage, wie `verify.py` sie bildet, gegengeprüft mit `verify_server_signature`. `ServerFinalMessage(client_first=…)` taugt dafür nicht: Bei Kanalbindung lässt es in `ecc1f7a` das `,,` hinter dem GS2-Kopf weg und lehnt dann den Beweis des eigenen Clients ab. Eingecheckt sind nur Ein- und Ausgaben, nicht das Skript und nicht die Bibliothek.
- Eingaben, alle ausgedacht: Benutzer `nasstats`, API-Key `1-<32 Buchstaben und Ziffern>`, Client-Nonce die Bytes 1 bis 32, Server-Nonce 101 bis 132, Salt 200 bis 215, 500000 Iterationen (Standard von TrueNAS).
- Zertifikat (`cert_der`, Base64): selbstsigniert mit `rcgen` 0.14.10 (`generate_simple_self_signed`, ECDSA P-256 mit SHA-256, Name `truenas.example`), ohne privaten Schlüssel. `cert_binding` ist der Wert von `compute_tls_server_end_point` der Bibliothek, also SHA-256 über das DER.

| Schlüssel | Inhalt |
|---|---|
| `unbound` | GS2-Kopf `n,,`, ohne Kanalbindung |
| `bound` | GS2-Kopf `p=tls-server-end-point,,` mit `cert_binding` |
| `client_first`, `client_final` | Nachrichten des Clients (`rfc_str`) |
| `server_first`, `server_final` | Antworten des Servers (`rfc_str`) |
| `server_final_signature` | Signatur aus `server_final` als Base64 |
