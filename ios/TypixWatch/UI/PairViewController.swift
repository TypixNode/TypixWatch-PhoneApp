import UIKit

/// The code from the watch's screen.  One wrong guess voids it on the watch, so the
/// button stays off until six digits are in.
final class PairViewController: ColumnViewController, WatchServiceObserver, UITextFieldDelegate {
    private let service: WatchService
    private let route: Route
    private let field = UITextField()
    private let button = ActionButton("配對")
    private let banner = ErrorBanner()

    init(service: WatchService, route: Route) {
        self.service = service
        self.route = route
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "輸入配對碼"
        column.addArrangedSubview(
            Theme.label("輸入手錶上顯示的六位數。輸錯一次這次配對就作廢，要回手錶重新開始。",
                        .bodyMedium, color: Theme.onSurfaceVariant))

        field.font = .monospacedDigitSystemFont(ofSize: 32, weight: .semibold)
        field.textColor = Theme.onSurface
        field.textAlignment = .center
        field.keyboardType = .numberPad
        field.textContentType = .none
        field.autocorrectionType = .no
        field.delegate = self
        field.defaultTextAttributes[.kern] = 8
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 64).isActive = true
        field.accessibilityLabel = "六位數配對碼"
        field.addAction(UIAction { [weak self] _ in self?.changed() }, for: .editingChanged)
        column.addArrangedSubview(Card([field]))

        button.isEnabled = false
        button.addAction(UIAction { [weak self] _ in self?.submit() }, for: .touchUpInside)
        column.addArrangedSubview(button)
        column.addArrangedSubview(banner)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        service.observer = self
        field.becomeFirstResponder()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if let code = TestDrive.code {
            field.text = code
            changed()
            submit()
        }
    }

    private func changed() {
        button.isEnabled = isValidPairingCode(code: field.text ?? "")
        banner.show(nil)
    }

    private func submit() {
        let code = field.text ?? ""
        guard isValidPairingCode(code: code) else { return }
        button.setBusy(true)
        field.isEnabled = false
        banner.show(nil)
        service.pair(via: route, code: code)
    }

    private func fail(_ text: String) {
        button.setBusy(false)
        field.isEnabled = true
        field.text = nil
        button.isEnabled = false
        banner.show(text)
    }

    func textField(
        _ textField: UITextField,
        shouldChangeCharactersIn range: NSRange,
        replacementString string: String
    ) -> Bool {
        guard string.allSatisfy({ $0.isASCII && $0.isNumber }) else { return false }
        let now = (textField.text ?? "") as NSString
        return now.replacingCharacters(in: range, with: string).count <= 6
    }

    func watchService(_ service: WatchService, didReceive event: WatchEvent) {
        switch event {
        case .paired:
            SceneDelegate.resetToRoot(from: view)
        case .failed(let error):
            fail(error.message)
        default:
            break
        }
    }

    func watchServiceCouldNotOpenLink(_ service: WatchService) {
        fail("連不上手錶。確認手錶已開啟，且在同一個網路或藍牙範圍內。")
    }
}
