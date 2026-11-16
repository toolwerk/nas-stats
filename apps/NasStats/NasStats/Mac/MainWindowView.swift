#if os(macOS)
import SwiftUI

/// What the sidebar of the main window shows.
enum MainSection: Hashable {
    case overview
    case history
    case events
    /// Detail page of one NAS, by its id.
    case nas(String)
}

/// Main window of the Mac (#28): a sidebar with Übersicht, Verlauf and
/// Ereignisse, below them every NAS with its status dot. The content uses
/// the views of the iPhone tabs; settings stay in their own window (⌘,).
struct MainWindowView: View {
    @Environment(NasStore.self) private var store
    @State private var selection: MainSection = .overview

    /// The list may clear its selection (⌘-click on the chosen row); an
    /// empty one keeps the current section, so sidebar and content agree.
    private var listSelection: Binding<MainSection?> {
        Binding(
            get: { selection },
            set: { if let section = $0 { selection = section } }
        )
    }

    var body: some View {
        NavigationSplitView {
            List(selection: listSelection) {
                Label("Übersicht", systemImage: "externaldrive")
                    .tag(MainSection.overview)
                Label("Verlauf", systemImage: "chart.xyaxis.line")
                    .tag(MainSection.history)
                Label("Ereignisse", systemImage: "bell")
                    .tag(MainSection.events)
                if !store.entries.isEmpty {
                    Section("NAS") {
                        ForEach(store.entries) { entry in
                            Label {
                                Text(verbatim: entry.displayName)
                            } icon: {
                                StatusDot(level: entry.statusLevel)
                            }
                            .tag(MainSection.nas(entry.id))
                        }
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 280)
        } detail: {
            detail
        }
        // A NAS removed meanwhile or gone with a switch of the demo mode
        // leaves its page for the overview.
        .onChange(of: store.entries.map(\.id)) { _, ids in
            if case .nas(let id) = selection, !ids.contains(id) {
                selection = .overview
            }
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .overview:
            OverviewView(onOpenNas: { selection = .nas($0) })
        case .history:
            HistoryView()
        case .events:
            EventsView()
        case .nas(let id):
            NavigationStack {
                NasDetailView(id: id)
            }
            // A new stack per NAS, so no page of the previous one stays.
            .id(id)
        }
    }
}
#endif
