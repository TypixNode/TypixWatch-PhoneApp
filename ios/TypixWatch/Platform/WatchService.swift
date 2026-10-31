import Foundation
import UIKit

/// How to reach the watch.
enum Route: Equatable {
    case tcp(String)
    case ble(UUID)
}

@MainActor
protocol WatchServiceObserver: AnyObject {
    func watchService(_ service: WatchService, didReceive event: WatchEvent)
    func watchServiceCouldNotOpenLink(_ service: WatchService)
}

/// The app's one `WatchClient`, and the links it runs on.  Observers hear events on the
/// main thread; bytes never pass through it.
@MainActor
final class WatchService {
    private static var instance: WatchService?

    static func shared() throws -> WatchService {
        if let s = instance { return s }
        let s = try WatchService()
        instance = s
        return s
    }

    let ble = BLECentral()
    weak var observer: WatchServiceObserver?
    private let client: WatchClient
    private let bridge: EventBridge
    private let worker = DispatchQueue(label: "typix.worker")
    private var timer: Timer?
    private var current: LinkHolder?
    private let manualHostKey = "manualHost"

    private init() throws {
        let bridge = EventBridge()
        self.bridge = bridge
        client = try WatchClient(store: KeychainStore(), events: bridge)
        bridge.owner = self
    }

    var isPaired: Bool { client.isPaired() }
    var state: ConnectionState { client.state() }
    var phoneFingerprint: String { client.phoneFingerprint() }
    var watchFingerprint: String? { client.watchFingerprint() }

    /// An address typed by hand wins over one the watch reported: it is what reaches
    /// the watch from here.
    var preferredHost: String? {
        UserDefaults.standard.string(forKey: manualHostKey) ?? client.savedHost()
    }

    func rememberManualHost(_ host: String) {
        UserDefaults.standard.set(host, forKey: manualHostKey)
    }

    func pair(via route: Route, code: String) {
        let name = UIDevice.current.name
        open(route) { [client] link, profile in
            try client.beginPairing(link: link, profile: profile, code: code, phoneName: name)
        }
    }

    func connect(via route: Route) {
        open(route) { [client] link, profile in
            try client.beginSession(link: link, profile: profile)
        }
    }

    func requestStatus() {
        try? client.requestStatus()
    }

    func disconnect() {
        client.disconnect()
    }

    /// `pushFile` blocks while it hashes, so it runs on `worker`.
    func push(_ data: Data, package: PackageType, name: String) {
        worker.async { [client] in
            do {
                try client.pushFile(package: package, name: name, source: MemorySource(data))
            } catch {
                let event = WatchEvent.failed(error: error as? CoreError ?? .LinkClosed)
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { WatchService.instance?.deliver(event) }
                }
            }
        }
    }

    func abortPush() {
        try? client.abortPush()
    }

    func pullLog(_ kind: LogKind, done: @escaping @Sendable (Data?) -> Void) throws {
        try client.pullLog(log: kind, sink: MemorySink(done: done))
    }

    func forgetWatch() throws {
        client.disconnect()
        try client.forgetWatch()
        UserDefaults.standard.removeObject(forKey: manualHostKey)
    }

    private func open(
        _ route: Route,
        begin: @escaping @Sendable (WatchLink, LinkProfile) throws -> Void
    ) {
        // Bytes from a link that was replaced must not reach the next session.
        current?.alive = false
        current?.link?.closeLink()
        let holder = LinkHolder()
        current = holder
        let received: @Sendable (Data) -> Void = { [client] data in
            if holder.alive { client.onBytes(bytes: data) }
        }
        let closed: @Sendable () -> Void = { [client] in
            if holder.alive { client.onLinkClosed() }
        }
        let ready: @Sendable (WatchLink?, LinkProfile) -> Void = { [weak self] link, profile in
            guard let link else {
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.linkFailed() }
                }
                return
            }
            holder.link = link
            do {
                try begin(link, profile)
            } catch {
                link.closeLink()
                let event = WatchEvent.failed(error: error as? CoreError ?? .LinkClosed)
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.deliver(event) }
                }
            }
        }
        startTimer()
        switch route {
        case .tcp(let host):
            var made: TCPLink?
            let tcp = TCPLink(
                host: host,
                opened: { ok in ready(ok ? made : nil, tcpProfile()) },
                received: received,
                closed: closed
            )
            made = tcp
            holder.keep = tcp
            tcp.start()
        case .ble(let id):
            ble.connect(
                id,
                opened: { link in ready(link, bleProfile()) },
                received: received,
                closed: closed
            )
        }
    }

    /// The core's only clock.
    private func startTimer() {
        guard timer == nil else { return }
        let client = client
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in client.tick() }
    }

    fileprivate func deliver(_ event: WatchEvent) {
        observer?.watchService(self, didReceive: event)
    }

    private func linkFailed() {
        observer?.watchServiceCouldNotOpenLink(self)
    }
}

private final class LinkHolder: @unchecked Sendable {
    var link: WatchLink?
    /// The TCP link before it has opened: nothing else holds it yet.
    var keep: AnyObject?
    var alive = true
}

private final class EventBridge: WatchEventListener, @unchecked Sendable {
    weak var owner: WatchService?

    /// The main queue keeps the events in order, which a `Task` would not promise.
    func onEvent(event: WatchEvent) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.owner?.deliver(event) }
        }
    }
}
