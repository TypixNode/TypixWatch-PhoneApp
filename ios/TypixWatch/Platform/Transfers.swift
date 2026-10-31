import Foundation

/// A file already in memory.  The watch takes 15 MB at most, so this stays small.
final class MemorySource: ByteSource, @unchecked Sendable {
    private let data: Data

    init(_ data: Data) { self.data = data }

    func size() throws -> UInt64 { UInt64(data.count) }

    func readAt(offset: UInt64, length: UInt32) throws -> Data {
        let start = data.startIndex + Int(offset)
        return data.subdata(in: start..<start + Int(length))
    }
}

/// Collects a log and hands it over only if its hash matched.  Runs under the core's
/// lock, so it only appends and posts.
final class MemorySink: ByteSink, @unchecked Sendable {
    private var data = Data()
    private let done: @Sendable (Data?) -> Void

    init(done: @escaping @Sendable (Data?) -> Void) { self.done = done }

    func write(bytes: Data) throws { data.append(bytes) }

    func finish(verified: Bool) { done(verified ? data : nil) }
}
