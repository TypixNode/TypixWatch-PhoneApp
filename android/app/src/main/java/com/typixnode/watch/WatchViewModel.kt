package com.typixnode.watch

import android.app.Application
import android.bluetooth.BluetoothDevice
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import com.typixnode.watch.platform.BleLink
import com.typixnode.watch.platform.BleScanner
import com.typixnode.watch.platform.FoundWatch
import com.typixnode.watch.platform.KeystoreStore
import com.typixnode.watch.platform.MemorySink
import com.typixnode.watch.platform.MemorySource
import com.typixnode.watch.platform.TcpLink
import java.util.concurrent.Executors
import uniffi.typix_core.ConnectionState
import uniffi.typix_core.CoreException
import uniffi.typix_core.LinkProfile
import uniffi.typix_core.LogKind
import uniffi.typix_core.PackageType
import uniffi.typix_core.WatchClient
import uniffi.typix_core.WatchEvent
import uniffi.typix_core.WatchEventListener
import uniffi.typix_core.WatchLink
import uniffi.typix_core.WatchStatus
import uniffi.typix_core.bleProfile
import uniffi.typix_core.tcpProfile

private const val WATCH_LIMIT = 15 * 1024 * 1024
private const val LOG_SHOWN = 32 * 1024
private const val MANUAL_HOST = "manualHost"

data class Transfer(val label: String, val done: ULong, val total: ULong)

/** `text` is null while the pull runs. */
data class LogView(val kind: LogKind, val text: String?)

sealed interface Route {
    data class Tcp(val host: String) : Route
    data class Ble(val device: BluetoothDevice) : Route
}

/** The app's one `WatchClient`.  State is read on the main thread; bytes never pass it. */
class WatchViewModel(app: Application) : AndroidViewModel(app), WatchEventListener {
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private val client = WatchClient(KeystoreStore(app), this)
    private val scanner = BleScanner(app) { w -> main.post { addFound(w) } }
    private var current: WatchLink? = null
    private var route: Route? = null
    private var lookingForWatch = false
    private val prefs = app.getSharedPreferences("typix", Context.MODE_PRIVATE)

    val found = mutableStateListOf<FoundWatch>()
    var paired by mutableStateOf(client.isPaired())
        private set
    var state by mutableStateOf(client.state())
        private set
    var status by mutableStateOf<WatchStatus?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var pairTarget by mutableStateOf<Route?>(null)
        private set
    var notice by mutableStateOf<String?>(null)
        private set
    var transfer by mutableStateOf<Transfer?>(null)
        private set
    var log by mutableStateOf<LogView?>(null)
        private set

    val phoneFingerprint: String get() = client.phoneFingerprint()
    val watchFingerprint: String? get() = client.watchFingerprint()
    /** An address typed by hand wins over one the watch reported: it is what reaches
     *  the watch from here. */
    val savedHost: String? get() = prefs.getString(MANUAL_HOST, null) ?: client.savedHost()

    private val ticker = object : Runnable {
        override fun run() {
            client.tick()
            main.postDelayed(this, 1000)
        }
    }

    init {
        main.post(ticker)
    }

    override fun onCleared() {
        main.removeCallbacks(ticker)
        client.disconnect()
    }

    private fun addFound(w: FoundWatch) {
        if (lookingForWatch) {
            lookingForWatch = false
            scanner.stop()
            route = Route.Ble(w.device)
            connect()
            return
        }
        if (found.none { it.device.address == w.device.address }) found.add(w)
    }

    fun startScan() {
        if (scanner.available) scanner.start()
    }

    fun stopScan() = scanner.stop()

    fun choose(route: Route) {
        stopScan()
        error = null
        if (route is Route.Tcp) prefs.edit().putString(MANUAL_HOST, route.host).apply()
        pairTarget = route
    }

    fun cancelPairing() {
        pairTarget = null
    }

    fun pair(code: String) {
        val target = pairTarget ?: return
        error = null
        route = target
        val name = Build.MODEL
        open(target) { link, profile -> client.beginPairing(link, profile, code, name) }
    }

    fun connect() {
        error = null
        val target = route ?: savedHost?.let { Route.Tcp(it) }
        if (target == null) {
            // No address known: the first watch advertising, the handshake proves which.
            lookingForWatch = true
            startScan()
            return
        }
        open(target) { link, profile -> client.beginSession(link, profile) }
    }

    fun refresh() {
        if (state == ConnectionState.READY && transfer == null) {
            runCatching { client.requestStatus() }
        }
    }

    /** Reads and hashes off the main thread; `pushFile` blocks while it hashes. */
    fun push(package_: PackageType, uri: Uri) {
        error = null
        val app = getApplication<Application>()
        transfer = Transfer("正在準備…", 0uL, 0uL)
        worker.execute {
            try {
                val data = app.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                    ?: throw IllegalArgumentException("讀不到檔案")
                require(data.size <= WATCH_LIMIT) { "超過手錶 15 MB 的上限" }
                client.pushFile(package_, displayName(uri), MemorySource(data))
            } catch (e: CoreException) {
                main.post { failTransfer(e.text()) }
            } catch (e: Exception) {
                main.post { failTransfer(e.message ?: e.toString()) }
            }
        }
    }

    fun abortPush() {
        runCatching { client.abortPush() }
    }

    private fun failTransfer(text: String) {
        transfer = null
        error = text
    }

    private fun displayName(uri: Uri): String {
        val resolver = getApplication<Application>().contentResolver
        val name = resolver.query(uri, null, null, null, null)?.use { c ->
            val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (i >= 0 && c.moveToFirst()) c.getString(i) else null
        }
        return name ?: "file"
    }

    fun pullLog(kind: LogKind) {
        error = null
        log = LogView(kind, null)
        val sink = MemorySink { bytes ->
            main.post {
                log = if (bytes == null) null else LogView(kind, tailOf(bytes))
            }
        }
        try {
            client.pullLog(kind, sink)
        } catch (e: CoreException) {
            log = null
            error = e.text()
        }
    }

    fun closeLog() {
        log = null
    }

    /** A screen of text, not a whole 256 KB dmesg. */
    private fun tailOf(bytes: ByteArray): String {
        val from = maxOf(0, bytes.size - LOG_SHOWN)
        return String(bytes, from, bytes.size - from, Charsets.UTF_8)
    }

    fun forget() {
        client.disconnect()
        client.forgetWatch()
        prefs.edit().remove(MANUAL_HOST).apply()
        paired = false
        status = null
        route = null
    }

    private fun open(target: Route, begin: (WatchLink, LinkProfile) -> Unit) {
        current?.closeLink()
        var link: WatchLink? = null
        val profile = if (target is Route.Tcp) tcpProfile() else bleProfile()
        val received = { b: ByteArray -> if (current === link) client.onBytes(b) }
        val closed = { if (current === link) client.onLinkClosed() }
        val opened: (Boolean) -> Unit = { ok ->
            val l = link
            if (!ok || l == null) {
                main.post { error = "連不上手錶。確認手錶已開啟，且在同一個網路或藍牙範圍內。" }
            } else {
                try {
                    begin(l, profile)
                } catch (e: CoreException) {
                    l.closeLink()
                    main.post { error = e.text() }
                }
            }
        }
        when (target) {
            is Route.Tcp -> TcpLink(target.host, opened, received, closed).also {
                link = it
                current = it
                it.start()
            }
            is Route.Ble -> BleLink(getApplication(), target.device, opened, received, closed).also {
                link = it
                current = it
                it.start()
            }
        }
    }

    override fun onEvent(event: WatchEvent) {
        main.post {
            when (event) {
                is WatchEvent.State -> {
                    state = event.state
                    if (event.state == ConnectionState.READY) refresh()
                }
                is WatchEvent.Paired -> {
                    paired = true
                    pairTarget = null
                }
                is WatchEvent.Status -> status = event.status
                is WatchEvent.Failed -> {
                    transfer = null
                    error = event.error.text()
                }
                is WatchEvent.PushStarted -> transfer = Transfer(
                    if (event.resumedFrom > 0uL) "從 ${event.resumedFrom} 續傳" else "傳送中",
                    event.resumedFrom,
                    0uL,
                )
                is WatchEvent.Progress -> transfer = transfer?.copy(
                    done = event.done,
                    total = event.total,
                )
                is WatchEvent.PushFinished -> {
                    transfer = null
                    notice = "已安裝：${event.result.message}"
                    refresh()
                }
                is WatchEvent.PushAborted -> {
                    transfer = null
                    notice = "已取消，手錶上的暫存已刪除"
                }
                is WatchEvent.PullFinished -> {}
            }
        }
    }
}

fun CoreException.text(): String = when (this) {
    is CoreException.InvalidCode -> "配對碼是六位數字。"
    is CoreException.PairingNotOpen -> "手錶不在配對狀態，或配對碼已過期。請到手錶上重新開始。"
    is CoreException.WrongCode -> "配對碼不對，這次配對已作廢。請到手錶上重新開始。"
    is CoreException.NotPaired -> "還沒配對。"
    is CoreException.SessionRejected -> "手錶拒絕連線：這支手機沒配對、已被撤銷，或手錶忙線中。"
    is CoreException.WatchKeyMismatch -> "手錶的金鑰和配對時不同，已停止連線。"
    is CoreException.NotConnected -> "尚未連線。"
    is CoreException.Busy -> "另一個操作還在進行。"
    is CoreException.Timeout -> "等手錶回覆逾時。"
    is CoreException.LinkClosed -> "連線中斷。手錶開放時段可能已結束。"
    is CoreException.HashMismatch -> "檔案校驗不符，沒有保留。"
    is CoreException.Remote -> "手錶回覆：$text"
    is CoreException.Protocol -> detail
    is CoreException.InvalidInput -> detail
    is CoreException.Storage -> detail
    is CoreException.File -> detail
    is CoreException.Crypto -> detail
}
