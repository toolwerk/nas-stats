# Datenschutz NAS Stats

Entwurf für `https://staude.net/nas-stats/datenschutz/`. Stand 2026-11-16.

## Wer

Frank Neumann-Staude, [Anschrift wie im Impressum von staude.net], frank@staude.net.

## Was die App liest

NAS Stats liest den Zustand der NAS, die Sie in der App eintragen. Gelesen werden:

- System: Modell, Firmware, CPU, Arbeitsspeicher, Temperaturen, Lüfter, Warnmeldungen,
- Volumes und Speicherpools mit Belegung und Größe,
- Platten mit Temperatur und SMART-Werten; die Seriennummer einer Platte speichert die App nur als Hashwert,
- Netzwerkanschlüsse mit Durchsatz,
- Container der Container Station, Backup-Aufgaben und eine angeschlossene USV.

Die App greift nur lesend zu. Sie sendet keine Befehle an ein NAS und ändert dort nichts.

## Wo die Daten bleiben

Es gibt keinen Server von NAS Stats. Die Daten Ihrer NAS liegen auf Ihren Geräten:

- Adresse, Benutzername und Passwort der NAS liegen im Schlüsselbund von iOS oder macOS. Standardmäßig teilt die App sie über den iCloud-Schlüsselbund mit Ihren anderen Geräten, damit Sie ein NAS nur einmal eintragen müssen; dafür gelten die Bedingungen von Apple. Der Schalter „Zugänge über iCloud-Schlüsselbund teilen“ in den Einstellungen (je Gerät) schaltet das aus.
- Der letzte Stand jedes NAS, die Einstellungen und der Verlauf (Belegung, Temperaturen, Ereignisse) liegen in einer Datenbank und in Dateien im Container der App. Die Widgets lesen daraus.
- Schalten Sie „Zustand und Verlauf in iCloud übertragen“ ein (je Gerät, standardmäßig aus), legt die Mac-App den letzten Stand, den Verlauf und die Ereignisse Ihrer NAS in Ihrer privaten iCloud-Datenbank ab, damit das iPhone sie unterwegs zeigt; Zugangsdaten gehen nicht mit. Ausschalten auf dem Mac löscht diese Daten aus iCloud.
- Es gibt keine Telemetrie, keine Analyse, keine Werbung und keine Weitergabe an Dritte.

## Mit wem die App spricht

- **Ihre eigenen NAS**, direkt über HTTPS. Beim ersten Kontakt merkt sich die App den Fingerabdruck des Zertifikats; ändert er sich, fragt sie nach.
- **Apple**: Systemdienste für Mitteilungen, Widgets, den iCloud-Schlüsselbund (solange Sie das Teilen der Zugänge nicht ausschalten) und die private iCloud-Datenbank (nur mit eingeschalteter Übertragung). NAS Stats nutzt sie nur über die Schnittstellen des Systems.

## Löschen

Entfernen Sie ein NAS in den Einstellungen, löscht die App dessen Zugang, den letzten Stand und den Verlauf. Löschen Sie die App, verschwinden alle lokalen Daten.

## Ihre Rechte

Da keine Daten bei uns liegen, gibt es bei uns nichts auszukünften oder zu löschen. Fragen zur App: frank@staude.net.

## Kinder

Die App richtet sich nicht an Kinder unter 16 Jahren.
