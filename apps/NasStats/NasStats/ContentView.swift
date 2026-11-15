import SwiftUI

/// Start screen of the iOS app until the first NAS is connected.
struct ContentView: View {
    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "externaldrive")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
            Text("NAS Stats")
                .font(.largeTitle.bold())
            Text("Noch kein NAS verbunden")
                .foregroundStyle(.secondary)
            Text(verbatim: "Core \(coreVersion())")
                .font(.footnote.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .padding()
    }
}

#Preview {
    ContentView()
}
