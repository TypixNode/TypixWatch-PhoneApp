import UIKit

/// One log off the watch.  Its text appears only after the hash matched.
final class LogViewController: UIViewController, WatchServiceObserver {
    private static let shown = 32 * 1024

    private let service: WatchService
    private let kind: LogKind
    private let text = UITextView()
    private let progress = Theme.label("下載中…", .bodyMedium, color: Theme.onSurfaceVariant)

    init(service: WatchService, kind: LogKind, title: String) {
        self.service = service
        self.kind = kind
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.surface
        text.isEditable = false
        text.backgroundColor = .clear
        text.textColor = Theme.onSurface
        text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.translatesAutoresizingMaskIntoConstraints = false
        progress.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(text)
        view.addSubview(progress)
        let g = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            progress.topAnchor.constraint(equalTo: g.topAnchor, constant: Theme.Space.s4),
            progress.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: Theme.Space.s4),
            progress.trailingAnchor.constraint(equalTo: g.trailingAnchor, constant: -Theme.Space.s4),
            text.topAnchor.constraint(equalTo: g.topAnchor),
            text.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            text.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: Theme.Space.s2),
            text.trailingAnchor.constraint(equalTo: g.trailingAnchor, constant: -Theme.Space.s2),
        ])
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        service.observer = self
        guard text.text.isEmpty else { return }
        do {
            try service.pullLog(kind) { [weak self] data in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.show(data) }
                }
            }
        } catch {
            progress.text = (error as? CoreError)?.message ?? "\(error)"
        }
    }

    /// A screen of text, not a whole 256 KB dmesg.
    private func show(_ data: Data?) {
        guard let data else { return }
        progress.isHidden = true
        text.text = String(decoding: data.suffix(Self.shown), as: UTF8.self)
        text.scrollRangeToVisible(NSRange(location: text.text.utf16.count, length: 0))
    }

    func watchService(_ service: WatchService, didReceive event: WatchEvent) {
        switch event {
        case .progress(let done, let total):
            progress.text = "下載中 \(done) / \(total)"
        case .failed(let error):
            progress.isHidden = false
            progress.text = error.message
        default:
            break
        }
    }

    func watchServiceCouldNotOpenLink(_ service: WatchService) {}
}
