package com.typixnode.watch.ui

import androidx.compose.foundation.background
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.typixnode.watch.Route
import com.typixnode.watch.WatchViewModel
import kotlinx.coroutines.delay
import uniffi.typix_core.ConnectionState
import uniffi.typix_core.LogKind
import uniffi.typix_core.PackageType
import uniffi.typix_core.isValidPairingCode

@Composable
fun App(vm: WatchViewModel) {
    Column(
        Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .safeDrawingPadding()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        when {
            vm.pairTarget != null -> PairScreen(vm)
            vm.log != null -> LogScreen(vm)
            vm.paired -> StatusScreen(vm)
            else -> ScanScreen(vm)
        }
    }
}

@Composable
private fun PanelCard(content: @Composable ColumnScope.() -> Unit) {
    Surface(shape = CardShape, color = MaterialTheme.colorScheme.surface) {
        Column(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
}

@Composable
private fun Action(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick,
        Modifier.fillMaxWidth().height(44.dp),
        enabled = enabled,
        shape = ButtonShape,
        colors = ButtonDefaults.buttonColors(),
    ) { Text(text, style = MaterialTheme.typography.titleSmall) }
}

@Composable
private fun Muted(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
private fun ErrorNote(text: String?) {
    if (text == null) return
    Surface(shape = ButtonShape, color = MaterialTheme.colorScheme.errorContainer) {
        Text(
            text,
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onErrorContainer,
        )
    }
}

@Composable
private fun InfoRow(name: String, value: String) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Muted(name)
        Text(value, style = MaterialTheme.typography.titleSmall)
    }
}

@Composable
private fun ScanScreen(vm: WatchViewModel) {
    var host by remember { mutableStateOf(vm.savedHost ?: "") }
    DisposableEffect(Unit) {
        vm.startScan()
        onDispose { vm.stopScan() }
    }
    Text("連接手錶", style = MaterialTheme.typography.headlineSmall)
    Muted("在手錶開啟「遠端開發」，選配對，再按大鍵。手錶會顯示六位數配對碼，有效 60 秒。")
    PanelCard {
        Text("藍牙", style = MaterialTheme.typography.titleSmall)
        if (vm.found.isEmpty()) Muted("正在尋找手錶…")
        vm.found.forEach { w ->
            OutlinedButton(
                { vm.choose(Route.Ble(w.device)) },
                Modifier.fillMaxWidth().height(44.dp),
                shape = ButtonShape,
            ) { Text(w.name) }
        }
    }
    PanelCard {
        Text("Wi-Fi", style = MaterialTheme.typography.titleSmall)
        OutlinedTextField(
            host,
            { host = it.trim() },
            Modifier.fillMaxWidth(),
            label = { Text("手錶的 IP 位址") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        )
        Action("下一步", enabled = host.isNotEmpty()) { vm.choose(Route.Tcp(host)) }
    }
}

@Composable
private fun PairScreen(vm: WatchViewModel) {
    var code by remember { mutableStateOf("") }
    Text("輸入配對碼", style = MaterialTheme.typography.headlineSmall)
    Muted("輸入手錶上顯示的六位數。輸錯一次這次配對就作廢，要回手錶重新開始。")
    PanelCard {
        OutlinedTextField(
            code,
            { v -> if (v.length <= 6 && v.all { it in '0'..'9' }) code = v },
            Modifier.fillMaxWidth(),
            textStyle = MaterialTheme.typography.headlineSmall,
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        )
    }
    val busy = vm.state != ConnectionState.IDLE
    Action("配對", enabled = isValidPairingCode(code) && !busy) { vm.pair(code) }
    TextButton({ vm.cancelPairing() }, Modifier.fillMaxWidth()) { Text("返回") }
    ErrorNote(vm.error)
}

@Composable
private fun StatusScreen(vm: WatchViewModel) {
    val ready = vm.state == ConnectionState.READY
    LaunchedEffect(Unit) {
        if (vm.state == ConnectionState.IDLE) vm.connect()
        // Only while on screen: every request holds the watch's window open.
        while (true) {
            delay(5000)
            vm.refresh()
        }
    }
    PanelCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (ready) {
                Spacer(Modifier.size(8.dp).background(MaterialTheme.colorScheme.tertiary, CircleShape))
                Spacer(Modifier.size(8.dp))
            }
            Text(
                when (vm.state) {
                    ConnectionState.READY -> "已連線"
                    ConnectionState.IDLE -> "尚未連線"
                    else -> "連線中…"
                },
                style = MaterialTheme.typography.titleMedium,
            )
        }
        if (vm.state == ConnectionState.IDLE) {
            Muted("要先在手錶開啟遠端開發並按大鍵。")
            Action("連線") { vm.connect() }
        }
    }
    ErrorNote(vm.error)
    vm.status?.let { s ->
        PanelCard {
            InfoRow("電量", s.batteryPercent?.let { "$it%" } ?: "讀不到")
            InfoRow("開放剩餘", "${s.secondsLeft / 60} 分鐘")
            InfoRow("Wi-Fi", s.wifiIp ?: "未連線")
            InfoRow("遠端 Shell", if (s.shellEnabled) "開啟" else "關閉")
        }
        PanelCard {
            InfoRow("OTA 空間", "${s.otaUsed / 1024u} / ${s.otaQuota / 1024u} KB")
            InfoRow("試用中", if (s.trial) "是" else "否")
            s.rollback?.let { InfoRow("曾回退", it) }
        }
    }
    if (ready) {
        TransferCard(vm)
        LogCard(vm)
    }
    PanelCard {
        Text("指紋", style = MaterialTheme.typography.titleSmall)
        InfoRow("手錶", vm.watchFingerprint ?: "—")
        InfoRow("這支手機", vm.phoneFingerprint)
        TextButton({ vm.forget() }, Modifier.fillMaxWidth()) {
            Text("忘記這支手錶", color = MaterialTheme.colorScheme.error)
        }
    }
}

private val PushKinds = listOf(
    "腳本" to PackageType.SCRIPT,
    "Launcher" to PackageType.LAUNCHER,
    "素材" to PackageType.ASSETS,
)

@Composable
private fun TransferCard(vm: WatchViewModel) {
    var kind by remember { mutableStateOf(PackageType.SCRIPT) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) vm.push(kind, uri)
    }
    PanelCard {
        Text("傳檔", style = MaterialTheme.typography.titleSmall)
        val t = vm.transfer
        if (t == null) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PushKinds.forEach { (label, value) ->
                    OutlinedButton({ kind = value }, shape = ButtonShape) {
                        Text(if (kind == value) "● $label" else label)
                    }
                }
            }
            Action("選擇檔案") { pick.launch(arrayOf("*/*")) }
            vm.notice?.let { Muted(it) }
        } else {
            Muted(t.label)
            if (t.total > 0uL) {
                LinearProgressIndicator(
                    { (t.done.toDouble() / t.total.toDouble()).toFloat() },
                    Modifier.fillMaxWidth(),
                )
                Muted("${t.done} / ${t.total}")
            }
            TextButton({ vm.abortPush() }) { Text("取消並刪除暫存") }
        }
    }
}

private val Logs = listOf(
    "Launcher" to LogKind.SHELL,
    "連線" to LogKind.CONNECTD,
    "OTA" to LogKind.OTAD,
    "核心" to LogKind.DMESG,
    "稽核" to LogKind.AUDIT,
)

@Composable
private fun LogCard(vm: WatchViewModel) {
    PanelCard {
        Text("Log", style = MaterialTheme.typography.titleSmall)
        Logs.forEach { (label, kind) ->
            OutlinedButton(
                { vm.pullLog(kind) },
                Modifier.fillMaxWidth().height(44.dp),
                enabled = vm.transfer == null,
                shape = ButtonShape,
            ) { Text(label) }
        }
    }
}

@Composable
private fun LogScreen(vm: WatchViewModel) {
    val view = vm.log ?: return
    Text("Log", style = MaterialTheme.typography.headlineSmall)
    TextButton({ vm.closeLog() }) { Text("返回") }
    if (view.text == null) {
        Muted("下載中…")
        vm.transfer?.let { Muted("${it.done} / ${it.total}") }
    } else {
        PanelCard {
            Text(
                view.text,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
        }
    }
}
