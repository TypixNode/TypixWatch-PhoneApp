import Foundation

/// Lets a script walk a Debug build through the app where nothing can tap, such as a
/// simulator driven over ssh.  `simctl launch` passes these as SIMCTL_CHILD_*.  Compiled
/// out of Release, and nothing here touches a key.
enum TestDrive {
    #if DEBUG
    private static let env = ProcessInfo.processInfo.environment
    static let host = env["TYPIX_TEST_HOST"]
    static let code = env["TYPIX_TEST_CODE"]
    static let pushesScript = env["TYPIX_TEST_PUSH"] != nil
    #else
    static let host: String? = nil
    static let code: String? = nil
    static let pushesScript = false
    #endif
}
