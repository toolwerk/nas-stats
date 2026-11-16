# Release

Ein Weg, aus der Shell, Xcode bleibt zu: `scripts/release.sh [ios|macos|all]` archiviert die Apps und lädt sie zu App Store Connect hoch (TestFlight). Standard ist `all`: erst iPhone, dann Mac (Mac App Store). Ein Mac-Zip außerhalb des Stores gibt es nicht.

Vor jedem Build, der das Haus verlässt: `scripts/bump-build.sh`, committen. Build-Nummern werden nie zurückgesetzt.

## Einmalige Einrichtung

1. **Zertifikate.** „Apple Distribution" für das Team `7N2W4T6UPV` im Anmelde-Schlüsselbund. Das Installer-Zertifikat für den Mac holt sich Xcode bei der automatischen Signierung (`-allowProvisioningUpdates`). Prüfen:

   ```sh
   security find-identity -v -p codesigning
   ```

2. **App Store Connect.** App-Einträge mit der Bundle-ID `net.staude.nasstats` für iOS und macOS anlegen (Name „NAS Stats", Primärsprache Deutsch). Eine Suche am 2026-11-16 fand keine App mit diesem Namen; endgültig entscheidet App Store Connect beim Anlegen. Die Widgets (`net.staude.nasstats.widgets`) sind Teil der App und brauchen keinen eigenen Eintrag.

3. **Hochladen.** Ohne weitere Einrichtung lädt das Skript mit dem in Xcode angemeldeten Konto hoch (Xcode › Einstellungen › Konten). Wer lieber einen API-Schlüssel nutzt (Rolle App Manager), legt die `.p8` nach `~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8` (oder setzt `ASC_KEY_PATH`) und ruft auf:

   ```sh
   ASC_KEY_ID=<key id> ASC_ISSUER_ID=<issuer id> scripts/release.sh
   ```

4. **Datenschutzseite** muss vor dem ersten Submit online sein: Entwurf in `docs/datenschutz.md`, Ziel `https://staude.net/nas-stats/datenschutz/`. Die Adresse kommt in App Store Connect unter „App-Datenschutz". Dort „Daten werden nicht erfasst" angeben.

5. **Store-Texte** aus `docs/store/texte.md` übernehmen (Deutsch und Englisch, mit Prüfhinweisen). Screenshots nur im Demo-Modus (Einstellungen › Demo-Modus), nie mit echten NAS: iPhone 6,9 Zoll, Mac 2880 × 1800.

## Ablauf

```sh
scripts/bump-build.sh
git commit -am "chore(release): build N"
scripts/release.sh            # oder ios / macos
```

Das Skript prüft zuerst xcodegen, einen sauberen Git-Stand und, falls gesetzt, den API-Schlüssel. Dann baut es den Rust-Core im Release-Profil, archiviert und lädt hoch. Die Verarbeitung in App Store Connect dauert einige Minuten, danach erscheint der Build in TestFlight. Eine Mail zum Datenschutz-Manifest darf nicht kommen; App und Widgets bringen `PrivacyInfo.xcprivacy` mit.

## App-Icon

Das Icon entsteht aus `scripts/icon/make-icon.swift` (NAS-Gehäuse mit Plattenschächten und grüner Zustandslampe auf dunklem Grund) und liegt in `apps/NasStats/NasStats/Assets.xcassets/AppIcon.appiconset`. Nach einer Änderung am Skript:

```sh
swift scripts/icon/make-icon.swift
```

Das iOS-Icon hat keinen Alphakanal (`sips -g hasAlpha` zeigt `no`), sonst lehnt App Store Connect es ab.

## Version

`MARKETING_VERSION` bleibt `0.1.0` bis zur Store-Einreichung.

## Was noch fehlt

Eine Einreichung im Store ist erst nach dem Verlauf (#19), der Ereignis-Chronik (#20) und dem Mac-Hauptfenster (#28) sinnvoll; der Mac zeigt bisher nur ein vorläufiges Einstellungsfenster. TestFlight geht schon vorher.
