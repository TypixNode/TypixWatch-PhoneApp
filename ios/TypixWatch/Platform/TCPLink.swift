import Foundation
import Network

/// The Wi-Fi and USB transport.  The core sees the same frames as over BLE.
final class TCPLink: WatchLink, @unchecked Sendable {
    private let queue = DispatchQueue(label: "typix.tcp")
    private let connection: NWConnection
    private var opened: ((Bool) -> Void)?
    private var closed: (() -> Void)?
    private let received: (Data) -> Void

    /// `opened` runs once.  `closed` runs once, only after a successful open.
    init(
        host: String,
        opened: @escaping (Bool) -> Void,
        received: @escaping (Data) -> Void,
        closed: @escaping () -> Void
    ) {
        let tcp = NWProtocolTCP.Options()
        tcp.noDelay = true
        tcp.connectionTimeout = 5
        connection = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: tcpPort())!,
            using: NWParameters(tls: nil, tcp: tcp)
        )
        self.opened = opened
        self.received = received
        self.closed = closed
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in self?.update(state) }
        connection.start(queue: queue)
        queue.asyncAfter(deadline: .now() + 6) { [weak self] in
            guard let self, self.opened != nil else { return }
            self.finishOpen(false)
            self.connection.cancel()
        }
    }

    private func update(_ state: NWConnection.State) {
        switch state {
        case .ready:
            finishOpen(true)
            receive()
        case .failed, .cancelled:
            if opened != nil { finishOpen(false) } else { finishClosed() }
        default:
            break
        }
    }

    private func finishOpen(_ ok: Bool) {
        let done = opened
        opened = nil
        if !ok { closed = nil }
        done?(ok)
    }

    private func finishClosed() {
        let done = closed
        closed = nil
        done?()
    }

    private func receive() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) {
            [weak self] data, _, complete, error in
            guard let self else { return }
            if let data, !data.isEmpty { self.received(data) }
            if complete || error != nil {
                self.connection.cancel()
            } else {
                self.receive()
            }
        }
    }

    func send(bytes: Data) {
        connection.send(content: bytes, completion: .contentProcessed { _ in })
    }

    func closeLink() {
        connection.cancel()
    }
}
