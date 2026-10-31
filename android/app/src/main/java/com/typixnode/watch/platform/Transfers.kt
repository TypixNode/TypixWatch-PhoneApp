package com.typixnode.watch.platform

import java.io.ByteArrayOutputStream
import uniffi.typix_core.ByteSink
import uniffi.typix_core.ByteSource

/** A file already in memory.  The watch takes 15 MB at most, so this stays small. */
class MemorySource(private val data: ByteArray) : ByteSource {
    override fun size(): ULong = data.size.toULong()

    override fun readAt(offset: ULong, length: UInt): ByteArray {
        val start = offset.toInt()
        return data.copyOfRange(start, start + length.toInt())
    }
}

/** Collects a log; hands it over only if its hash matched. */
class MemorySink(private val done: (ByteArray?) -> Unit) : ByteSink {
    private val out = ByteArrayOutputStream()

    override fun write(bytes: ByteArray) = out.write(bytes)

    override fun finish(verified: Boolean) = done(if (verified) out.toByteArray() else null)
}
