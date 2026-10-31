package com.typixnode.watch.platform

import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors
import uniffi.typix_core.WatchLink
import uniffi.typix_core.tcpPort

/** The Wi-Fi and USB transport: the same frames as BLE, over a socket. */
class TcpLink(
    private val host: String,
    private val opened: (Boolean) -> Unit,
    private val received: (ByteArray) -> Unit,
    private val closed: () -> Unit,
) : WatchLink {
    private val socket = Socket()
    private val writer = Executors.newSingleThreadExecutor()

    fun start() {
        Thread({ run() }, "typix-tcp").start()
    }

    private fun run() {
        try {
            socket.tcpNoDelay = true
            socket.connect(InetSocketAddress(host, tcpPort().toInt()), 5000)
        } catch (e: IOException) {
            opened(false)
            writer.shutdown()
            return
        }
        opened(true)
        val buffer = ByteArray(64 * 1024)
        try {
            val input = socket.getInputStream()
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                received(buffer.copyOf(n))
            }
        } catch (_: IOException) {
        }
        writer.shutdown()
        closed()
    }

    /** One writer thread, so frames leave in call order. */
    override fun send(bytes: ByteArray) {
        writer.execute {
            try {
                socket.getOutputStream().write(bytes)
            } catch (_: IOException) {
                closeLink()
            }
        }
    }

    override fun closeLink() {
        try {
            socket.close()
        } catch (_: IOException) {
        }
    }
}
