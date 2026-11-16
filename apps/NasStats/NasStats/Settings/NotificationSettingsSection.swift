import SwiftUI

/// Notification part of the settings (#39): a switch per kind of problem
/// and the quiet hours.
struct NotificationSettingsSection: View {
    @Environment(NasStore.self) private var store
    /// Notifications switched off in the system settings; read on appear.
    @State private var denied = false

    var body: some View {
        Section {
            ForEach(NotificationCategory.allCases, id: \.self) { category in
                Toggle(category.title, isOn: enabled(category))
            }
            Toggle("Ruhezeit", isOn: quiet(\.enabled))
            if store.notificationSettings.quietHours.enabled {
                DatePicker("Von", selection: time(\.startMinute), displayedComponents: .hourAndMinute)
                DatePicker("Bis", selection: time(\.endMinute), displayedComponents: .hourAndMinute)
            }
        } header: {
            Text("Mitteilungen")
        } footer: {
            footer
        }
        .task { denied = await store.notificationsDenied() }
    }

    @ViewBuilder private var footer: some View {
        if denied {
            Text("Mitteilungen sind in den Systemeinstellungen ausgeschaltet.")
        } else {
            #if os(iOS)
            Text("Je Problem eine Mitteilung, erneut erst, wenn es zwischendurch erledigt war. In der Ruhezeit hält NAS Stats Mitteilungen bis zu ihrem Ende zurück. Auf dem iPhone nur, wenn iOS die App im Hintergrund abfragen lässt.")
            #else
            Text("Je Problem eine Mitteilung, erneut erst, wenn es zwischendurch erledigt war. In der Ruhezeit hält NAS Stats Mitteilungen bis zu ihrem Ende zurück.")
            #endif
        }
    }

    private func enabled(_ category: NotificationCategory) -> Binding<Bool> {
        Binding(
            get: { store.notificationSettings.isEnabled(category) },
            set: { on in
                if on {
                    store.notificationSettings.disabled.remove(category)
                } else {
                    store.notificationSettings.disabled.insert(category)
                }
            }
        )
    }

    private func quiet(_ path: WritableKeyPath<QuietHours, Bool>) -> Binding<Bool> {
        Binding(
            get: { store.notificationSettings.quietHours[keyPath: path] },
            set: { store.notificationSettings.quietHours[keyPath: path] = $0 }
        )
    }

    /// Minutes after midnight as a time of today, for the picker.
    private func time(_ path: WritableKeyPath<QuietHours, Int>) -> Binding<Date> {
        let calendar = Calendar.current
        return Binding(
            get: {
                let minute = store.notificationSettings.quietHours[keyPath: path]
                return calendar.date(
                    bySettingHour: minute / 60, minute: minute % 60, second: 0, of: Date()
                ) ?? Date()
            },
            set: { date in
                let parts = calendar.dateComponents([.hour, .minute], from: date)
                store.notificationSettings.quietHours[keyPath: path] =
                    (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
            }
        )
    }
}
