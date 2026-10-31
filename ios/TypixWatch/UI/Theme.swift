import UIKit

/// One2X tokens, from the design system's tokens.json: semantic colors in light and dark,
/// the type scale, the radii and the spacing.  Black is the default action; purple
/// appears at most once on a screen.  Colors are named by role, never by hex at the call site.
enum Theme {
    private static func dynamic(_ light: UInt32, _ dark: UInt32) -> UIColor {
        UIColor { $0.userInterfaceStyle == .dark ? rgb(dark) : rgb(light) }
    }

    private static func rgb(_ v: UInt32) -> UIColor {
        UIColor(
            red: CGFloat(v >> 16 & 255) / 255,
            green: CGFloat(v >> 8 & 255) / 255,
            blue: CGFloat(v & 255) / 255,
            alpha: 1
        )
    }

    static let primary = dynamic(0x863dfb, 0x863dfb)
    static let surface = dynamic(0xf4f4f5, 0x17141a)
    static let card = dynamic(0xffffff, 0x26222a)
    static let onSurface = dynamic(0x09090b, 0xe5e3e8)
    static let onSurfaceVariant = dynamic(0x3f3f46, 0xa49daf)
    static let outline = dynamic(0xd4d4d8, 0x413d48)
    static let outlineVariant = dynamic(0xe4e4e7, 0x3c3842)
    /// The default action's fill.
    static let inverseSurface = dynamic(0x09090b, 0xe0ccff)
    static let inverseOnSurface = dynamic(0xffffff, 0x313031)
    static let error = dynamic(0xba1a1a, 0xffb4ab)
    static let errorContainer = dynamic(0xffdad6, 0x93000a)
    static let onErrorContainer = dynamic(0x410002, 0xffdad6)

    enum Space {
        static let s1: CGFloat = 4
        static let s2: CGFloat = 8
        static let s3: CGFloat = 12
        static let s4: CGFloat = 16
        static let s6: CGFloat = 24
        static let s7: CGFloat = 32
    }

    enum Radius {
        static let button: CGFloat = 12
        static let card: CGFloat = 16
    }

    enum Text {
        case headlineSmall, titleLarge, titleMedium, titleSmall
        case labelLarge, bodyMedium, bodySmall
        case code

        var size: CGFloat {
            switch self {
            case .headlineSmall: return 24
            case .titleLarge: return 22
            case .titleMedium, .code: return 16
            case .titleSmall, .labelLarge, .bodyMedium: return 14
            case .bodySmall: return 12
            }
        }

        var semibold: Bool {
            switch self {
            case .headlineSmall, .titleLarge, .titleMedium, .titleSmall: return true
            default: return false
            }
        }
    }

    /// Manrope when `scripts/build-ios.sh` could bundle it, otherwise the system face.
    /// Headlines want Nohemi, whose license keeps it out of this repository.
    static func font(_ style: Text) -> UIFont {
        let name = style.semibold ? "Manrope3-Semibold" : "Manrope3-Medium"
        let weight: UIFont.Weight = style.semibold ? .semibold : .medium
        let base = UIFont(name: name, size: style.size)
            ?? .systemFont(ofSize: style.size, weight: weight)
        return UIFontMetrics.default.scaledFont(for: base)
    }

    static func label(_ text: String = "", _ style: Text, color: UIColor = Theme.onSurface) -> UILabel {
        let l = UILabel()
        l.text = text
        l.font = font(style)
        l.textColor = color
        l.numberOfLines = 0
        l.adjustsFontForContentSizeCategory = true
        return l
    }
}

/// The filled button.  44 points tall where the token says 40: this is a touch target.
final class ActionButton: UIButton {
    enum Kind { case filled, outlined, destructive }

    init(_ title: String, kind: Kind = .filled) {
        super.init(frame: .zero)
        var c: UIButton.Configuration = kind == .filled ? .filled() : .plain()
        c.cornerStyle = .fixed
        c.background.cornerRadius = Theme.Radius.button
        c.title = title
        c.titleTextAttributesTransformer = .init { a in
            var a = a
            a.font = Theme.font(.titleSmall)
            return a
        }
        switch kind {
        case .filled:
            c.baseBackgroundColor = Theme.inverseSurface
            c.baseForegroundColor = Theme.inverseOnSurface
        case .outlined:
            c.baseForegroundColor = Theme.onSurface
            c.background.strokeColor = Theme.outline
            c.background.strokeWidth = 1
        case .destructive:
            c.baseForegroundColor = Theme.error
        }
        configuration = c
        heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// A small arc in place of the title, as the design system's loading state.
    func setBusy(_ busy: Bool) {
        configuration?.showsActivityIndicator = busy
        isEnabled = !busy
    }
}

/// A white panel on the neutral surface.
final class Card: UIStackView {
    init(_ views: [UIView] = [], spacing: CGFloat = Theme.Space.s3) {
        super.init(frame: .zero)
        axis = .vertical
        self.spacing = spacing
        isLayoutMarginsRelativeArrangement = true
        directionalLayoutMargins = .init(
            top: Theme.Space.s4, leading: Theme.Space.s4,
            bottom: Theme.Space.s4, trailing: Theme.Space.s4)
        backgroundColor = Theme.card
        layer.cornerRadius = Theme.Radius.card
        layer.cornerCurve = .continuous
        views.forEach(addArrangedSubview)
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }
}

/// A labelled value, one line of a status card.
final class InfoRow: UIStackView {
    private let value = Theme.label("—", .titleSmall)

    init(_ title: String) {
        super.init(frame: .zero)
        spacing = Theme.Space.s3
        alignment = .firstBaseline
        let name = Theme.label(title, .bodyMedium, color: Theme.onSurfaceVariant)
        name.setContentHuggingPriority(.required, for: .horizontal)
        name.setContentCompressionResistancePriority(.required, for: .horizontal)
        value.textAlignment = .right
        addArrangedSubview(name)
        addArrangedSubview(value)
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    func set(_ text: String) { value.text = text }
}

/// The inline error, under whatever caused it.
final class ErrorBanner: UIView {
    private let label = Theme.label("", .bodyMedium, color: Theme.onErrorContainer)

    init() {
        super.init(frame: .zero)
        backgroundColor = Theme.errorContainer
        layer.cornerRadius = Theme.Radius.button
        layer.cornerCurve = .continuous
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: topAnchor, constant: Theme.Space.s3),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -Theme.Space.s3),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Theme.Space.s4),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Theme.Space.s4),
        ])
        isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(_ text: String?) {
        label.text = text
        isHidden = text == nil
    }
}

/// A scrolling column, the layout of every screen.
class ColumnViewController: UIViewController {
    let column = UIStackView()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.surface
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = true
        scroll.keyboardDismissMode = .interactive
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        column.axis = .vertical
        column.spacing = Theme.Space.s4
        column.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(column)
        let g = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            column.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: Theme.Space.s4),
            column.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -Theme.Space.s7),
            column.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: Theme.Space.s4),
            column.trailingAnchor.constraint(equalTo: g.trailingAnchor, constant: -Theme.Space.s4),
        ])
    }
}

/// Shown when the app cannot start, instead of crashing.
final class NoticeViewController: ColumnViewController {
    private let heading: String
    private let message: String

    init(title: String, message: String) {
        heading = title
        self.message = message
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        column.addArrangedSubview(Theme.label(heading, .headlineSmall))
        column.addArrangedSubview(Theme.label(message, .bodyMedium, color: Theme.onSurfaceVariant))
    }
}

extension CoreError {
    /// What the person should read, and do.
    var message: String {
        switch self {
        case .InvalidCode: return "配對碼是六位數字。"
        case .PairingNotOpen: return "手錶不在配對狀態，或配對碼已過期。請到手錶上重新開始。"
        case .WrongCode: return "配對碼不對，這次配對已作廢。請到手錶上重新開始。"
        case .NotPaired: return "還沒配對。"
        case .SessionRejected: return "手錶拒絕連線：這支手機沒配對、已被撤銷，或手錶忙線中。"
        case .WatchKeyMismatch: return "手錶的金鑰和配對時不同，已停止連線。"
        case .NotConnected: return "尚未連線。"
        case .Busy: return "另一個操作還在進行。"
        case .Timeout: return "等手錶回覆逾時。"
        case .LinkClosed: return "連線中斷。手錶開放時段可能已結束。"
        case .HashMismatch: return "檔案校驗不符，沒有保留。"
        case .Remote(_, let text): return "手錶回覆：\(text)"
        case .Protocol(let d), .InvalidInput(let d), .Storage(let d), .File(let d), .Crypto(let d):
            return d
        }
    }
}
