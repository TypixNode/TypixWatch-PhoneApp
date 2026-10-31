import UIKit
import UniformTypeIdentifiers

/// What the watch says about itself while its window is open.
final class StatusViewController: ColumnViewController, WatchServiceObserver,
    UIDocumentPickerDelegate {
    private static let watchLimit = 15 * 1024 * 1024
    private static let logs: [(String, LogKind)] = [
        ("Launcher", .shell), ("連線", .connectd), ("OTA", .otad), ("核心", .dmesg), ("稽核", .audit),
    ]
    private static let kinds: [PackageType] = [.script, .launcher, .assets]

    private let service: WatchService
    private let dot = UIView()
    private let stateLabel = Theme.label("尚未連線", .titleMedium)
    private let hint = Theme.label("", .bodySmall, color: Theme.onSurfaceVariant)
    private let banner = ErrorBanner()
    private let battery = InfoRow("電量")
    private let window = InfoRow("開放剩餘")
    private let wifi = InfoRow("Wi-Fi")
    private let shell = InfoRow("遠端 Shell")
    private let storage = InfoRow("OTA 空間")
    private let storageBar = UIProgressView(progressViewStyle: .bar)
    private let launcher = InfoRow("Launcher")
    private let assets = InfoRow("素材")
    private let trial = InfoRow("試用中")
    private let rollback = InfoRow("曾回退")
    private let watchID = InfoRow("手錶")
    private let phoneID = InfoRow("這支手機")
    private let connectButton = ActionButton("連線")
    private let kindPicker = UISegmentedControl(items: ["腳本", "Launcher", "素材"])
    private let pickButton = ActionButton("選擇檔案")
    private let transferLabel = Theme.label("", .bodyMedium, color: Theme.onSurfaceVariant)
    private let transferBar = UIProgressView(progressViewStyle: .bar)
    private let abortButton = ActionButton("取消並刪除暫存", kind: .destructive)
    private let transferCard = Card()
    private let logCard = Card(spacing: Theme.Space.s2)
    private var transferring = false
    private var testPushed = false
    private var poll: Timer?
    private var scanningForWatch = false

    init(service: WatchService) {
        self.service = service
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Typix Watch"

        dot.backgroundColor = Theme.primary
        dot.layer.cornerRadius = 4
        dot.isHidden = true
        NSLayoutConstraint.activate([
            dot.widthAnchor.constraint(equalToConstant: 8),
            dot.heightAnchor.constraint(equalToConstant: 8),
        ])
        let top = UIStackView(arrangedSubviews: [dot, stateLabel])
        top.spacing = Theme.Space.s2
        top.alignment = .center

        connectButton.addAction(UIAction { [weak self] _ in self?.connect() }, for: .touchUpInside)
        column.addArrangedSubview(Card([top, hint, connectButton]))
        column.addArrangedSubview(banner)
        column.addArrangedSubview(Card([battery, window, wifi, shell]))

        storageBar.trackTintColor = Theme.outlineVariant
        storageBar.progressTintColor = Theme.inverseSurface
        storageBar.layer.cornerRadius = 2
        storageBar.clipsToBounds = true
        storageBar.heightAnchor.constraint(equalToConstant: 4).isActive = true
        column.addArrangedSubview(Card([storage, storageBar, launcher, assets, trial, rollback]))

        kindPicker.selectedSegmentIndex = 0
        pickButton.addAction(UIAction { [weak self] _ in self?.pickFile() }, for: .touchUpInside)
        abortButton.addAction(UIAction { [weak self] _ in self?.service.abortPush() }, for: .touchUpInside)
        transferBar.trackTintColor = Theme.outlineVariant
        transferBar.progressTintColor = Theme.inverseSurface
        transferBar.heightAnchor.constraint(equalToConstant: 4).isActive = true
        [Theme.label("傳檔", .titleSmall), kindPicker, pickButton, transferLabel, transferBar, abortButton]
            .forEach(transferCard.addArrangedSubview)
        column.addArrangedSubview(transferCard)
        setTransferring(false)

        logCard.addArrangedSubview(Theme.label("Log", .titleSmall))
        for (title, kind) in Self.logs {
            let b = ActionButton(title, kind: .outlined)
            b.addAction(UIAction { [weak self] _ in self?.open(kind, title) }, for: .touchUpInside)
            logCard.addArrangedSubview(b)
        }
        column.addArrangedSubview(logCard)

        watchID.set(service.watchFingerprint ?? "—")
        phoneID.set(service.phoneFingerprint)
        let forget = ActionButton("忘記這支手錶", kind: .destructive)
        forget.addAction(UIAction { [weak self] _ in self?.confirmForget() }, for: .touchUpInside)
        column.addArrangedSubview(Card([Theme.label("指紋", .titleSmall), watchID, phoneID, forget]))
        rollback.isHidden = true
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        service.observer = self
        render(service.state)
        if service.state == .ready {
            service.requestStatus()
        } else {
            connect()
        }
        // Polls only while on screen: every request holds the watch's window open.
        poll = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        poll?.invalidate()
        stopLooking()
    }

    private func tick() {
        if service.state == .ready, !transferring { service.requestStatus() }
    }

    private func connect() {
        guard service.state == .idle else { return }
        banner.show(nil)
        if let host = service.preferredHost {
            connectButton.setBusy(true)
            service.connect(via: .tcp(host))
        } else {
            lookOverBluetooth()
        }
    }

    /// No address known: take the first watch advertising, the handshake proves which.
    private func lookOverBluetooth() {
        connectButton.setBusy(true)
        hint.text = "正在用藍牙尋找手錶…"
        scanningForWatch = true
        let ble = service.ble
        ble.onFound = { [weak self] w in
            DispatchQueue.main.async {
                guard let self, self.scanningForWatch else { return }
                self.stopLooking()
                self.service.connect(via: .ble(w.id))
            }
        }
        ble.onAvailability = { [weak self] a in
            DispatchQueue.main.async {
                guard let self, self.scanningForWatch, a != .on, a != .unknown else { return }
                self.stopLooking()
                self.connectButton.setBusy(false)
                self.hint.text = nil
                self.askForAddress()
            }
        }
        ble.startScan()
    }

    private func stopLooking() {
        scanningForWatch = false
        service.ble.stopScan()
    }

    private func askForAddress() {
        let a = UIAlertController(
            title: "用 Wi-Fi 連線", message: "這台裝置沒有可用的藍牙。輸入手錶的 IP 位址。",
            preferredStyle: .alert)
        a.addTextField { $0.keyboardType = .numbersAndPunctuation }
        a.addAction(UIAlertAction(title: "取消", style: .cancel))
        a.addAction(UIAlertAction(title: "連線", style: .default) { [weak self, weak a] _ in
            guard let self, let h = a?.textFields?.first?.text, !h.isEmpty else { return }
            self.service.rememberManualHost(h)
            self.connect()
        })
        present(a, animated: true)
    }

    private func pickFile() {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.data], asCopy: true)
        picker.delegate = self
        present(picker, animated: true)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let url = urls.first else { return }
        banner.show(nil)
        guard let data = try? Data(contentsOf: url) else {
            banner.show("讀不到檔案。")
            return
        }
        guard data.count <= Self.watchLimit else {
            banner.show("超過手錶 15 MB 的上限。")
            return
        }
        setTransferring(true)
        transferLabel.text = "正在準備…"
        let kind = Self.kinds[kindPicker.selectedSegmentIndex]
        service.push(data, package: kind, name: url.lastPathComponent)
    }

    /// Once, for `TestDrive`: the document picker needs a finger.
    private func runTestPush() {
        guard TestDrive.pushesScript, !testPushed else { return }
        testPushed = true
        setTransferring(true)
        service.push(Data("echo typix\n".utf8), package: .script, name: "ios-e2e.sh")
    }

    private func setTransferring(_ on: Bool) {
        transferring = on
        kindPicker.isHidden = on
        pickButton.isHidden = on
        transferBar.isHidden = !on
        abortButton.isHidden = !on
        transferBar.progress = 0
        transferLabel.isHidden = transferLabel.text == nil && !on
    }

    private func finishTransfer(_ note: String?) {
        transferLabel.text = note
        setTransferring(false)
    }

    private func open(_ kind: LogKind, _ title: String) {
        navigationController?.pushViewController(
            LogViewController(service: service, kind: kind, title: title), animated: true)
    }

    private func confirmForget() {
        let a = UIAlertController(
            title: "忘記這支手錶？",
            message: "手機上的配對會刪除。手錶上的紀錄要在手錶撤銷。",
            preferredStyle: .alert)
        a.addAction(UIAlertAction(title: "取消", style: .cancel))
        a.addAction(UIAlertAction(title: "忘記", style: .destructive) { [weak self] _ in
            guard let self else { return }
            do {
                try self.service.forgetWatch()
                SceneDelegate.resetToRoot(from: self.view)
            } catch {
                self.banner.show((error as? CoreError)?.message ?? "\(error)")
            }
        })
        present(a, animated: true)
    }

    private func render(_ state: ConnectionState) {
        let ready = state == .ready
        dot.isHidden = !ready
        transferCard.isHidden = !ready
        logCard.isHidden = !ready
        connectButton.isHidden = ready
        connectButton.setBusy(state == .handshaking)
        switch state {
        case .ready:
            stateLabel.text = "已連線"
            hint.text = "手錶閒置 10 分鐘或開啟滿 30 分鐘會自動關閉。"
        case .handshaking, .pairing:
            stateLabel.text = "連線中…"
            hint.text = nil
        case .idle:
            stateLabel.text = "尚未連線"
            hint.text = "要先在手錶開啟遠端開發並按大鍵。"
        }
        hint.isHidden = hint.text == nil
    }

    private func render(_ s: WatchStatus) {
        battery.set(s.batteryPercent.map { "\($0)%" } ?? "讀不到")
        window.set(Self.minutes(s.secondsLeft))
        wifi.set(s.wifiIp ?? "未連線")
        shell.set(s.shellEnabled ? "開啟" : "關閉")
        storage.set("\(Self.size(s.otaUsed)) / \(Self.size(s.otaQuota))")
        storageBar.progress = s.otaQuota == 0 ? 0 : Float(s.otaUsed) / Float(s.otaQuota)
        launcher.set(s.overlayLauncher == 0 ? "映像內建" : "已更新 \(Self.size(s.overlayLauncher))")
        assets.set(s.overlayAssets == 0 ? "映像內建" : "已更新 \(Self.size(s.overlayAssets))")
        trial.set(s.trial ? "是" : "否")
        rollback.set(s.rollback ?? "")
        rollback.isHidden = s.rollback == nil
    }

    private static func minutes(_ s: Int64) -> String {
        s >= 60 ? "\(s / 60) 分鐘" : "\(max(s, 0)) 秒"
    }

    private static func size(_ n: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(n), countStyle: .binary)
    }

    func watchService(_ service: WatchService, didReceive event: WatchEvent) {
        switch event {
        case .state(let state):
            render(state)
            if state == .ready { service.requestStatus() }
        case .status(let status):
            render(status)
            runTestPush()
        case .pushStarted(let from):
            transferLabel.text = from > 0 ? "從 \(from) 續傳" : "傳送中"
        case .progress(let done, let total):
            transferBar.progress = total == 0 ? 0 : Float(done) / Float(total)
            transferLabel.text = "\(done) / \(total)"
        case .pushFinished(let result):
            finishTransfer("已安裝：\(result.message)")
            service.requestStatus()
            if TestDrive.pushesScript { open(.audit, "稽核") }
        case .pushAborted:
            finishTransfer("已取消，手錶上的暫存已刪除")
        case .failed(let error):
            finishTransfer(nil)
            render(service.state)
            connectButton.setBusy(false)
            banner.show(error.message)
        default:
            break
        }
    }

    func watchServiceCouldNotOpenLink(_ service: WatchService) {
        render(service.state)
        connectButton.setBusy(false)
        banner.show("連不上手錶。確認手錶已開啟遠端開發，且在同一個網路或藍牙範圍內。")
    }
}
