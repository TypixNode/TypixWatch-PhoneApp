import Foundation
import Security

/// The core's key storage.  `ThisDeviceOnly` keeps the items out of backups and iCloud:
/// the two private keys equal root on the watch, and a new device pairs again.
final class KeychainStore: SecureStore, @unchecked Sendable {
    private let service = "com.typixnode.TypixWatch.keys"

    private func query(_ name: String) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: name,
        ]
    }

    func load(name: String) throws -> Data? {
        var q = query(name)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw failure("read", status) }
        return item as? Data
    }

    func save(name: String, value: Data) throws {
        let attributes: [CFString: Any] = [
            kSecValueData: value,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        var status = SecItemUpdate(query(name) as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            let add = query(name).merging(attributes) { _, new in new }
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw failure("write", status) }
    }

    func remove(name: String) throws {
        let status = SecItemDelete(query(name) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw failure("delete", status)
        }
    }

    private func failure(_ what: String, _ status: OSStatus) -> CoreError {
        .Storage(detail: "keychain \(what): \(status)")
    }
}
