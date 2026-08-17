import AppKit
import Testing
@testable import CodexBar

@MainActor
struct SettingsWindowOpeningTests {
    @Test
    func `keepalive shell stays invisible and inert to Mission Control`() {
        let keepaliveShell = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 500),
            styleMask: [.titled],
            backing: .buffered,
            defer: false)
        let configuratorView = KeepaliveWindowConfiguratorView(windowProvider: { _ in keepaliveShell })
        configuratorView.viewDidMoveToWindow()

        #expect(keepaliveShell.identifier?.rawValue == "CodexBarLifecycleKeepalive")
        #expect(keepaliveShell.styleMask == [.borderless])
        #expect(keepaliveShell.level == .normal)
        #expect(keepaliveShell.collectionBehavior == [.auxiliary, .ignoresCycle, .transient])
        #expect(!keepaliveShell.collectionBehavior.contains(.canJoinAllSpaces))
        #expect(keepaliveShell.isExcludedFromWindowsMenu)
        #expect(!keepaliveShell.isOpaque)
        #expect(keepaliveShell.alphaValue == 0)
        #expect(!keepaliveShell.hasShadow)
        #expect(keepaliveShell.ignoresMouseEvents)
        #expect(!keepaliveShell.canHide)
        #expect(keepaliveShell.frame.size == NSSize(width: 1, height: 1))
        #expect(keepaliveShell.frame.origin == NSPoint(x: -5000, y: -5000))
    }

    @Test
    func `missing keepalive relay invokes settings fallback`() {
        let settingsWindow = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 480),
            styleMask: [.titled],
            backing: .buffered,
            defer: false)

        var presentedWindow: NSWindow?
        var prepareCount = 0
        let opener = SettingsWindowOpener(
            prepare: { prepareCount += 1 },
            notification: { false },
            appKit: {
                presentedWindow = settingsWindow
                return true
            })

        let outcome = opener.open(preferred: .notification)

        #expect(outcome == .fallback)
        #expect(prepareCount == 1)
        #expect(presentedWindow === settingsWindow)
    }
}
