#if os(macOS)
import AppKit
import Observation
import SwiftUI

/// The NAS symbol in the menu bar (#27): coloured by the worst NAS, with
/// fill level or warning count if chosen. A left click opens the popover
/// (#28), a right click the actions. It also owns the windows: the main
/// window with the sidebar (#28), adding a NAS and the settings.
///
/// AppKit instead of `MenuBarExtra`: only a status item tells left and
/// right clicks apart and shows a coloured image.
@MainActor
final class StatusItemController: NSObject, NSWindowDelegate {
    private let store: NasStore
    private let defaults: UserDefaults
    private let statusItem: NSStatusItem
    private let popover = NSPopover()
    private var mainWindow: NSWindow?
    private var addNasWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var defaultsObserver: NSObjectProtocol?

    init(store: NasStore, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()
        // Closes on a click outside, like a menu.
        popover.behavior = .transient
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
        guard right else {
            togglePopover(sender)
            return
        }
        popover.performClose(nil)
        // The menu hangs on the item only for this click; otherwise every
        // click would open it and the action would never fire.
        statusItem.menu = actionsMenu()
        sender.performClick(nil)
        statusItem.menu = nil
    }

    /// Left click: closes an open popover, else opens it with a new view, so
    /// it starts at the worst NAS instead of the last one chosen.
    private func togglePopover(_ button: NSStatusBarButton) {
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        let view = MenuBarPopover(
            onOpenWindow: { [weak self] in
                self?.popover.performClose(nil)
                self?.openMainWindow()
            },
            onAddNas: { [weak self] in
                self?.popover.performClose(nil)
                self?.openAddNas()
            }
        )
            .environment(store)
        let controller = NSHostingController(rootView: view)
        controller.sizingOptions = [.preferredContentSize]
        popover.contentViewController = controller
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        // Key, so the buttons take the first click.
        popover.contentViewController?.view.window?.makeKey()
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

    private func action(_ title: String, _ selector: Selector, key: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        return item
    }

    // MARK: - Actions

    /// Queries only NAS whose last query is at least the interval of the
    /// settings ago (5 minutes or more, #21).
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
        // A new view each time, so the form starts empty. It closes its own
        // window: a test query that ends after the window was closed must not
        // close a newer one.
        let own = WindowReference()
        let view = AddNasView(
            onClose: { own.window?.close() },
            isOpen: { own.window?.isVisible == true }
        )
            .environment(store)
        let window = makeWindow(title: "NAS hinzufügen", root: view)
        own.window = window
        addNasWindow = window
    }

    @objc private func openSettings() {
        showSettings()
    }

    /// The settings window, also for ⌘, of the app menu while the main
    /// window is in front.
    func showSettings() {
        if let settingsWindow {
            bringToFront(settingsWindow)
            return
        }
        let view = SettingsView(onAddNas: { [weak self] in self?.openAddNas() })
            .environment(store)
        settingsWindow = makeWindow(title: "Einstellungen", root: view)
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    // MARK: - Windows

    /// Main window with the sidebar (#28). While it is open the app shows
    /// in the Dock and in ⌘-Tab, so it can be found behind other windows;
    /// after closing it is a menu bar app again.
    func openMainWindow() {
        NSApp.setActivationPolicy(.regular)
        if let mainWindow {
            bringToFront(mainWindow)
            return
        }
        let controller = NSHostingController(rootView: MainWindowView().environment(store))
        // Title and the toolbar items of the views go to the window.
        controller.sceneBridgingOptions = [.toolbars, .title]
        let window = NSWindow(contentViewController: controller)
        window.title = "NAS Stats"
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.setContentSize(NSSize(width: 960, height: 640))
        window.minSize = NSSize(width: 760, height: 480)
        window.isReleasedWhenClosed = false
        window.delegate = self
        // Takes the frame of the last time, if there is one.
        if !window.setFrameUsingName(Self.mainWindowFrameName) {
            window.center()
        }
        window.setFrameAutosaveName(Self.mainWindowFrameName)
        mainWindow = window
        bringToFront(window)
    }

    private static let mainWindowFrameName = "NasStatsMainWindow"

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
        if window === mainWindow {
            mainWindow = nil
            NSApp.setActivationPolicy(.accessory)
        } else if window === addNasWindow {
            addNasWindow = nil
        } else if window === settingsWindow {
            settingsWindow = nil
        }
    }
}

/// Weak link from a view to the window it shows in.
@MainActor
private final class WindowReference {
    weak var window: NSWindow?
}
#endif
