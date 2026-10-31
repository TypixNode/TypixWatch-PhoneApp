import UIKit

/// First screen: find the watch, by Bluetooth or by address.
final class ScanViewController: ColumnViewController, WatchServiceObserver, UITextFieldDelegate {
    private let service: WatchService
    private var found: [FoundWatch] = []
    private let bleStatus = Theme.label("", .bodyMedium, color: Theme.onSurfaceVariant)
    private let list = UIStackView()
    private let hostField = UITextField()

    init(service: WatchService) {
        self.service = service
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "連接手錶"

        let step = Theme.label(
            "在手錶開啟「遠端開發」，選配對，再按大鍵。手錶會顯示六位數配對碼，有效 60 秒。",
            .bodyMedium, color: Theme.onSurfaceVariant)
        column.addArrangedSubview(step)

        list.axis = .vertical
        list.spacing = Theme.Space.s2
        column.addArrangedSubview(Card([Theme.label("藍牙", .titleSmall), bleStatus, list]))

        hostField.placeholder = "手錶的 IP 位址"
        hostField.font = Theme.font(.bodyMedium)
        hostField.keyboardType = .numbersAndPunctuation
        hostField.autocorrectionType = .no
        hostField.autocapitalizationType = .none
        hostField.returnKeyType = .next
        hostField.delegate = self
        hostField.borderStyle = .none
        hostField.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        hostField.text = service.preferredHost

        let next = ActionButton("下一步")
        next.addAction(UIAction { [weak self] _ in self?.useHost() }, for: .touchUpInside)
        column.addArrangedSubview(Card([Theme.label("Wi-Fi", .titleSmall), hostField, next]))
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        service.observer = self
        let ble = service.ble
        ble.onAvailability = { [weak self] a in DispatchQueue.main.async { self?.show(a) } }
        ble.onFound = { [weak self] w in DispatchQueue.main.async { self?.add(w) } }
        ble.startScan()
        show(.unknown)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if let host = TestDrive.host {
            hostField.text = host
            useHost()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        service.ble.stopScan()
    }

    private func show(_ a: BLEAvailability) {
        switch a {
        case .unknown, .on: bleStatus.text = found.isEmpty ? "正在尋找手錶…" : nil
        case .off: bleStatus.text = "藍牙關著。到系統設定開啟，或改用 Wi-Fi。"
        case .denied: bleStatus.text = "沒有藍牙權限。到系統設定允許，或改用 Wi-Fi。"
        case .unsupported: bleStatus.text = "這台裝置沒有藍牙。模擬器請改用 Wi-Fi。"
        }
        bleStatus.isHidden = bleStatus.text == nil
    }

    private func add(_ w: FoundWatch) {
        if let i = found.firstIndex(where: { $0.id == w.id }) {
            found[i] = w
            return
        }
        found.append(w)
        show(.on)
        let row = ActionButton(w.name, kind: .outlined)
        row.addAction(UIAction { [weak self] _ in self?.go(.ble(w.id)) }, for: .touchUpInside)
        list.addArrangedSubview(row)
    }

    private func useHost() {
        let host = hostField.text?.trimmingCharacters(in: .whitespaces) ?? ""
        guard !host.isEmpty else {
            hostField.becomeFirstResponder()
            return
        }
        service.rememberManualHost(host)
        go(.tcp(host))
    }

    private func go(_ route: Route) {
        navigationController?.pushViewController(
            PairViewController(service: service, route: route), animated: true)
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        useHost()
        return true
    }

    func watchService(_ service: WatchService, didReceive event: WatchEvent) {}
    func watchServiceCouldNotOpenLink(_ service: WatchService) {}
}
