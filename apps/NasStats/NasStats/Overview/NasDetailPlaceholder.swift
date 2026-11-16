import SwiftUI

/// Stand-in for the detail page of a NAS until #18.
struct NasDetailPlaceholder: View {
    let name: String

    var body: some View {
        ContentUnavailableView(
            name,
            systemImage: "externaldrive",
            description: Text("Die Detailseite folgt.")
        )
        .navigationTitle(name)
    }
}

#Preview {
    NavigationStack {
        NasDetailPlaceholder(name: "Keller")
    }
}
