#if os(macOS)
import AppKit
import Observation
import SwiftUI

/// The NAS symbol in the menu bar (#27): coloured by the worst NAS, with
/// fill level or warning count if chosen. A left click lists the NAS, a
/// right click opens the actions.
///
/// AppKit instead of `MenuBarExtra`: only a status item tells left and
/// right clicks apart and shows a coloured image. #28 puts the popover on
/// the left click.
@MainActor
final class StatusItemController: NSObject, NSWindowDelegate {
    private let store: NasStore
    private let defaults: UserDefaults
    private let statusItem: NSStatusItem
    private var addNasWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var defaultsObserver: NSObjectProtocol?

    init(store: NasStore, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()
        if let button = statusItem.button {
            button.target = self
            button.action = #selector(clicked(_:))
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        // The settings window changes the display through the defaults.
        defaultsObserver = NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification, object: defaults, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.redraw() }
        }
        observeStore()
    }

    private var display: MenuBarDisplay {
        get { MenuBarDisplay(stored: defaults.string(forKey: MenuBarDisplay.defaultsKey)) }
        set {
            defaults.set(newValue.rawValue, forKey: MenuBarDisplay.defaultsKey)
            redraw()
        }
    }

    // MARK: - Symbol

    /// Draws the symbol and draws it again after every change of the store.
    private func observeStore() {
        withObservationTracking {
            redraw()
        } onChange: { [weak self] in
            Task { @MainActor in self?.observeStore() }
        }
    }

    private func redraw() {
        guard let button = statusItem.button else { return }
        let summary = MenuBarSummary(entries: store.entries, demo: store.demoMode)
        var image = NSImage(systemSymbolName: "externaldrive.fill", accessibilityDescription: nil)
        if let level = summary.symbolLevel {
            let colour = NSImage.SymbolConfiguration(paletteColors: [NSColor(level.color)])
            image = image?.withSymbolConfiguration(colour)
            image?.isTemplate = false
        } else {
            image?.isTemplate = true
        }
        button.image = image
        let text = summary.text(for: display)
        button.title = text ?? ""
        button.imagePosition = text == nil ? .imageOnly : .imageLeading
        button.toolTip = summary.tooltip
        button.setAccessibilityLabel(summary.tooltip)
    }

    // MARK: - Clicks

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        let right = event?.type == .rightMouseUp
            || event?.modifierFlags.contains(.control) == true
        // The menu hangs on the item only for this click; otherwise every
        // click would open it and the action would never fire.
        statusItem.menu = right ? actionsMenu() : listMenu()
        sender.performClick(nil)
        statusItem.menu = nil
    }

    /// Left click, provisional until the popover of #28: the NAS, worst first.
    private func listMenu() -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.addItem(info(store.demoMode ? "NAS Stats – Demo-Modus" : "NAS Stats"))
        if let loadError = store.loadError {
            menu.addItem(info(loadError))
        }
        if store.entries.isEmpty {
            if store.loadError == nil {
                menu.addItem(info("Noch kein NAS verbunden"))
            }
        } else {
            for entry in store.entries {
                let item = NSMenuItem(
                    title: "\(entry.displayName) – \(entry.health.title)", action: nil, keyEquivalent: ""
                )
                item.image = NSImage(systemSymbolName: entry.health.symbol, accessibilityDescription: nil)
                menu.addItem(item)
            }
        }
        menu.addItem(.separator())
        menu.addItem(action("NAS hinzufügen …", #selector(openAddNas)))
        return menu
    }

    /// Right click: refresh, display, login item, settings, quit.
    private func actionsMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(action("Aktualisieren", #selector(refresh), key: "r"))
        menu.addItem(.separator())

        let displayItem = NSMenuItem(title: "Anzeige", action: nil, keyEquivalent: "")
        let displayMenu = NSMenu()
        let current = display
        for choice in MenuBarDisplay.allCases {
            let item = action(choice.title, #selector(chooseDisplay(_:)))
            item.representedObject = choice.rawValue
            item.state = choice == current ? .on : .off
            displayMenu.addItem(item)
        }
        displayItem.submenu = displayMenu
        menu.addItem(displayItem)
        menu.addItem(.separator())

        let login = action("Bei Anmeldung starten", #selector(toggleLaunchAtLogin))
        login.state = LaunchAtLogin.isEnabled ? .on : .off
        menu.addItem(login)
        menu.addItem(action("Einstellungen …", #selector(openSettings), key: ","))
        menu.addItem(.separator())
        menu.addItem(action("NAS Stats beenden", #selector(quit), key: "q"))
        return menu
    }

    private func info(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }

    private func action(_ title: String, _ selector: Selector, key: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        return item
    }

    // MARK: - Actions

    /// Queries only NAS whose last query is at least 5 minutes ago.
    @objc private func refresh() {
        Task { await store.refreshAll() }
    }

    @objc private func chooseDisplay(_ sender: NSMenuItem) {
        display = MenuBarDisplay(stored: sender.representedObject as? String)
    }

    @objc private func toggleLaunchAtLogin() {
        LaunchAtLogin.toggle()
    }

    @objc private func openAddNas() {
        if let addNasWindow {
            bringToFront(addNasWindow)
            return
        }
        // A new view each time, so the form starts empty.
        let view = AddNasView(onClose: { [weak self] in self?.addNasWindow?.close() })
            .environment(store)
        addNasWindow = makeWindow(title: "NAS hinzufügen", root: view)
    }

    @objc private func openSettings() {
        if let settingsWindow {
            bringToFront(settingsWindow)
            return
        }
        let view = MacSettingsView().environment(store)
        settingsWindow = makeWindow(title: "Einstellungen", root: view)
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    // MARK: - Windows

    private func makeWindow(title: String, root: some View) -> NSWindow {
        let controller = NSHostingController(rootView: root)
        controller.sizingOptions = [.preferredContentSize]
        let window = NSWindow(contentViewController: controller)
        window.title = title
        window.styleMask = [.titled, .closable]
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()
        bringToFront(window)
        return window
    }

    private func bringToFront(_ window: NSWindow) {
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        if window === addNasWindow {
            addNasWindow = nil
        } else if window === settingsWindow {
            settingsWindow = nil
        }
    }
}
#endif
