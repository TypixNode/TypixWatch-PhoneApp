import CoreBluetooth
import Foundation

/// A watch seen advertising.  Identity comes from the handshake, never from this.
struct FoundWatch: Equatable {
    let id: UUID
    let name: String
    var rssi: Int
}

enum BLEAvailability: Equatable {
    case unknown, on, off, denied
    /// The simulator ends up here: it has no Bluetooth.
    case unsupported
}

/// CoreBluetooth for the core: finds the watch, opens the GATT pipe, moves bytes.
/// Link-layer pairing is not used; all encryption is above this.
final class BLECentral: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate, @unchecked Sendable {
    private let serviceID = CBUUID(string: bleServiceUuid())
    private let rxID = CBUUID(string: bleRxUuid())
    private let txID = CBUUID(string: bleTxUuid())
    private let queue = DispatchQueue(label: "typix.ble")
    private var central: CBCentralManager!
    private var seen: [UUID: CBPeripheral] = [:]
    private var scanning = false
    private var link: BLELink?

    var onAvailability: ((BLEAvailability) -> Void)?
    var onFound: ((FoundWatch) -> Void)?

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: queue)
    }

    func startScan() {
        queue.async {
            self.scanning = true
            self.beginScanIfReady()
        }
    }

    func stopScan() {
        queue.async {
            self.scanning = false
            if self.central.state == .poweredOn { self.central.stopScan() }
        }
    }

    private func beginScanIfReady() {
        guard scanning, central.state == .poweredOn else { return }
        central.scanForPeripherals(withServices: [serviceID])
    }

    func connect(
        _ id: UUID,
        opened: @escaping (WatchLink?) -> Void,
        received: @escaping (Data) -> Void,
        closed: @escaping () -> Void
    ) {
        queue.async {
            guard let p = self.seen[id] else {
                opened(nil)
                return
            }
            self.central.stopScan()
            let link = BLELink(central: self.central, peripheral: p, queue: self.queue)
            link.opened = opened
            link.received = received
            link.closed = closed
            self.link = link
            p.delegate = self
            self.central.connect(p)
            self.queue.asyncAfter(deadline: .now() + 20) { [weak link] in
                link?.failOpen()
            }
        }
    }

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        let a: BLEAvailability
        switch c.state {
        case .poweredOn: a = .on
        case .poweredOff: a = .off
        case .unauthorized: a = .denied
        case .unsupported: a = .unsupported
        default: a = .unknown
        }
        onAvailability?(a)
        beginScanIfReady()
    }

    func centralManager(
        _ c: CBCentralManager,
        didDiscover p: CBPeripheral,
        advertisementData: [String: Any],
        rssi: NSNumber
    ) {
        seen[p.identifier] = p
        let name = advertisementData[CBAdvertisementDataLocalNameKey] as? String ?? p.name ?? "W1A"
        onFound?(FoundWatch(id: p.identifier, name: name, rssi: rssi.intValue))
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        p.discoverServices([serviceID])
    }

    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        link?.failOpen()
    }

    func centralManager(
        _ c: CBCentralManager,
        didDisconnectPeripheral p: CBPeripheral,
        error: Error?
    ) {
        link?.didClose()
        link = nil
    }

    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        guard let s = p.services?.first(where: { $0.uuid == serviceID }) else {
            link?.failOpen()
            return
        }
        p.discoverCharacteristics([rxID, txID], for: s)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        guard let rx = s.characteristics?.first(where: { $0.uuid == rxID }),
              let tx = s.characteristics?.first(where: { $0.uuid == txID }) else {
            link?.failOpen()
            return
        }
        link?.rx = rx
        p.setNotifyValue(true, for: tx)
    }

    func peripheral(
        _ p: CBPeripheral,
        didUpdateNotificationStateFor c: CBCharacteristic,
        error: Error?
    ) {
        if error == nil, c.isNotifying { link?.didOpen() } else { link?.failOpen() }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor c: CBCharacteristic, error: Error?) {
        if let v = c.value { link?.received?(v) }
    }

    func peripheralIsReady(toSendWriteWithoutResponse p: CBPeripheral) {
        link?.pump()
    }
}

/// One GATT connection as a byte pipe.  All state lives on the central's queue.
final class BLELink: WatchLink, @unchecked Sendable {
    private let central: CBCentralManager
    private let peripheral: CBPeripheral
    private let queue: DispatchQueue
    private var pending = Data()
    fileprivate var rx: CBCharacteristic?
    fileprivate var opened: ((WatchLink?) -> Void)?
    fileprivate var received: ((Data) -> Void)?
    fileprivate var closed: (() -> Void)?

    fileprivate init(central: CBCentralManager, peripheral: CBPeripheral, queue: DispatchQueue) {
        self.central = central
        self.peripheral = peripheral
        self.queue = queue
    }

    fileprivate func didOpen() {
        let done = opened
        opened = nil
        done?(self)
    }

    fileprivate func failOpen() {
        guard let done = opened else { return }
        opened = nil
        closed = nil
        central.cancelPeripheralConnection(peripheral)
        done(nil)
    }

    fileprivate func didClose() {
        if opened != nil {
            failOpen()
            return
        }
        let done = closed
        closed = nil
        done?()
    }

    func send(bytes: Data) {
        queue.async {
            self.pending.append(bytes)
            self.pump()
        }
    }

    /// Writes without response, cut to the MTU, as far as CoreBluetooth has room.
    fileprivate func pump() {
        guard let rx, peripheral.state == .connected else { return }
        let max = peripheral.maximumWriteValueLength(for: .withoutResponse)
        while !pending.isEmpty, peripheral.canSendWriteWithoutResponse {
            let piece = pending.prefix(max)
            peripheral.writeValue(Data(piece), for: rx, type: .withoutResponse)
            pending.removeFirst(piece.count)
        }
    }

    func closeLink() {
        queue.async { self.central.cancelPeripheralConnection(self.peripheral) }
    }
}
