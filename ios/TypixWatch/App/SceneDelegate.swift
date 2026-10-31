import UIKit

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        window.tintColor = Theme.primary
        window.rootViewController = Self.root()
        window.makeKeyAndVisible()
        self.window = window
    }

    /// The whole app is a stack with one root, chosen by whether a watch is pinned.
    static func root() -> UIViewController {
        let top: UIViewController
        do {
            let service = try WatchService.shared()
            top = service.isPaired ? StatusViewController(service: service) : ScanViewController(service: service)
        } catch {
            top = NoticeViewController(title: "無法讀取金鑰", message: "\(error)")
        }
        let nav = UINavigationController(rootViewController: top)
        nav.navigationBar.tintColor = Theme.onSurface
        nav.navigationBar.titleTextAttributes = [
            .font: Theme.font(.titleMedium), .foregroundColor: Theme.onSurface,
        ]
        return nav
    }

    static func resetToRoot(from view: UIView?) {
        guard let window = view?.window else { return }
        UIView.transition(with: window, duration: 0.25, options: .transitionCrossDissolve) {
            window.rootViewController = root()
        }
    }
}
