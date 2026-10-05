package dev.audiobridge.app

import android.Manifest
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import dev.audiobridge.app.capture.CaptureService
import dev.audiobridge.app.play.PlaybackService
import kotlinx.coroutines.delay

class MainActivity : ComponentActivity() {

    private lateinit var projectionLauncher: androidx.activity.result.ActivityResultLauncher<android.content.Intent>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Settings.init(this)

        requestNotifPermissionIfNeeded()

        projectionLauncher = registerForActivityResult(
            ActivityResultContracts.StartActivityForResult(),
        ) { result ->
            val data = result.data
            if (result.resultCode == RESULT_OK && data != null) {
                CaptureService.start(this, result.resultCode, data)
            } else {
                AppBus.updateSender { it.copy(error = "未获得「录制/投射音频」授权，无法采集系统声音") }
            }
        }

        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    MainScreen(
                        onStartSend = {
                            if (Settings.testSource) {
                                CaptureService.startTest(this)
                            } else {
                                requestProjection()
                            }
                        },
                        onStopSend = { CaptureService.stop(this) },
                        onStartReceive = { PlaybackService.start(this) },
                        onStopReceive = { PlaybackService.stop(this) },
                    )
                }
            }
        }
    }

    private fun requestNotifPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            registerForActivityResult(ActivityResultContracts.RequestPermission()) {}.launch(
                Manifest.permission.POST_NOTIFICATIONS,
            )
        }
    }

    private fun requestProjection() {
        val sm = getSystemService(MediaProjectionManager::class.java)
        projectionLauncher.launch(sm.createScreenCaptureIntent())
    }
}

@Composable
fun MainScreen(
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    onStartReceive: () -> Unit,
    onStopReceive: () -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var tab by remember { mutableIntStateOf(0) }

    // —— 更新状态（头部"检查更新"与横幅共享）——
    var updateInfo by remember { mutableStateOf<dev.audiobridge.app.util.AppUpdater.Info?>(null) }
    var updatePhase by remember { mutableStateOf("idle") } // idle | downloading | ready
    var updateProgress by remember { mutableStateOf(0f) }
    var updateStatus by remember { mutableStateOf("") }
    var downloadedFile by remember { mutableStateOf<java.io.File?>(null) }
    var checkHint by remember { mutableStateOf("") }

    fun manualCheck() {
        checkHint = "检查更新中…"
        Thread {
            val r = runCatching { dev.audiobridge.app.util.AppUpdater.check(context) }.getOrNull()
            if (r != null) {
                if (Settings.ignoredUpdateVersion == r.version) {
                    Settings.ignoredUpdateVersion = "" // 手动检查视为明确想看
                }
                updateInfo = r
                checkHint = ""
            } else {
                checkHint = "已是最新版本 v${dev.audiobridge.app.util.AppUpdater.currentVersion(context)}"
            }
        }.start()
    }

    // 启动时自动检查（用户忽略过的版本不打扰）
    LaunchedEffect(Unit) {
        Thread {
            val r = runCatching { dev.audiobridge.app.util.AppUpdater.check(context) }.getOrNull()
            if (r != null && r.version != Settings.ignoredUpdateVersion) {
                updateInfo = r
            }
        }.start()
    }

    LaunchedEffect(checkHint) {
        if (checkHint.isNotEmpty()) {
            delay(4000)
            checkHint = ""
        }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(start = 16.dp, end = 8.dp),
        ) {
            Text(
                text = "AudioBridge 音频桥",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f),
            )
            androidx.compose.material3.TextButton(onClick = { manualCheck() }) {
                Text("检查更新")
            }
        }
        if (checkHint.isNotEmpty()) {
            Text(
                checkHint,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(start = 16.dp, bottom = 4.dp),
            )
        }

        val cur = updateInfo
        if (cur != null && cur.version != Settings.ignoredUpdateVersion) {
            UpdateBanner(
                info = cur,
                phase = updatePhase,
                progress = updateProgress,
                statusText = updateStatus,
                onDownload = {
                    updatePhase = "downloading"
                    updateStatus = "开始下载…"
                    Thread {
                        try {
                            val f = dev.audiobridge.app.util.AppUpdater.download(
                                context, cur.androidUrl,
                            ) { got, total ->
                                if (total > 0) {
                                    updateProgress = got.toFloat() / total
                                    updateStatus = "后台下载中 %.1f / %.1f MB".format(
                                        got / 1048576.0, total / 1048576.0,
                                    )
                                }
                            }
                            downloadedFile = f
                            updatePhase = "ready"
                        } catch (e: Exception) {
                            updatePhase = "idle"
                            checkHint = "更新下载失败: ${e.message}"
                        }
                    }.start()
                },
                onInstall = {
                    downloadedFile?.let {
                        runCatching {
                            dev.audiobridge.app.util.AppUpdater.installApk(context, it)
                        }
                    }
                },
                onIgnore = {
                    Settings.ignoredUpdateVersion = cur.version
                },
            )
        }

        TabRow(selectedTabIndex = tab) {
            Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("发送到电脑") })
            Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("接收电脑声音") })
        }
        when (tab) {
            0 -> SenderPane(onStartSend, onStopSend)
            1 -> ReceiverPane(onStartReceive, onStopReceive)
        }
    }
}

@Composable
private fun UpdateBanner(
    info: dev.audiobridge.app.util.AppUpdater.Info,
    phase: String,
    progress: Float,
    statusText: String,
    onDownload: () -> Unit,
    onInstall: () -> Unit,
    onIgnore: () -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    Surface(color = Color(0xFFFFF3D6), modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(12.dp)) {
            when (phase) {
                "ready" -> {
                    Text(
                        "新版本 v${info.version} 已下载完成（当前 v${
                            dev.audiobridge.app.util.AppUpdater.currentVersion(context)
                        }）",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Button(onClick = onInstall) { Text("安装更新") }
                        Spacer(Modifier.width(8.dp))
                        Text(
                            "调起系统安装器，按提示确认",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
                "downloading" -> {
                    Text(
                        "正在后台下载更新…（可继续使用，完成后在此提示）",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 6.dp),
                    )
                    if (statusText.isNotEmpty()) {
                        Text(statusText, style = MaterialTheme.typography.bodySmall)
                    }
                }
                else -> {
                    Text(
                        "发现新版本 v${info.version}（当前 v${
                            dev.audiobridge.app.util.AppUpdater.currentVersion(context)
                        }）",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    if (info.notes.isNotBlank()) {
                        Text(info.notes, style = MaterialTheme.typography.bodySmall)
                    }
                    Row {
                        Button(onClick = onDownload) { Text("更新（后台下载）") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = onIgnore) { Text("忽略此版本") }
                    }
                }
            }
        }
    }
}

@Composable
private fun SenderPane(onStart: () -> Unit, onStop: () -> Unit) {
    val state by AppBus.sender.collectAsState()
    var host by remember { mutableStateOf(Settings.host) }
    var port by remember { mutableStateOf(Settings.port.toString()) }
    var testSource by remember { mutableStateOf(Settings.testSource) }
    var scanning by remember { mutableStateOf(false) }
    var devices by remember { mutableStateOf(emptyList<dev.audiobridge.app.proto.Discovery.DiscoveredDevice>()) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
    ) {
        Text("把本机的系统声音（电视剧、音乐等）实时发送到局域网内的电脑。", style = MaterialTheme.typography.bodyMedium)

        Spacer(Modifier.height(12.dp))
        // —— 自动发现 ——
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = {
                    val p = port.toIntOrNull() ?: 48000
                    scanning = true
                    Thread {
                        val r = dev.audiobridge.app.proto.Discovery.scanBlocking(p, 2500)
                        devices = r
                        scanning = false
                    }.start()
                },
                enabled = !scanning,
            ) { Text(if (scanning) "扫描中…" else "扫描局域网设备") }
            Spacer(Modifier.width(12.dp))
            Text(
                if (devices.isEmpty()) "未发现设备（对方需先启动接收）" else "发现 ${devices.size} 台，点击「连接」",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        devices.forEach { d ->
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 4.dp),
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(d.name, style = MaterialTheme.typography.bodyMedium)
                    Text("${d.addr}:${d.tcpPort}", style = MaterialTheme.typography.bodySmall)
                }
                Button(
                    onClick = {
                        host = d.addr
                        port = d.tcpPort.toString()
                        Settings.host = d.addr
                        Settings.port = d.tcpPort
                        onStart()
                    },
                    enabled = !state.running,
                ) { Text("连接") }
            }
        }

        Spacer(Modifier.height(8.dp))
        Text("手动指定地址（通常无需填写）", style = MaterialTheme.typography.labelMedium)
        OutlinedTextField(
            value = host,
            onValueChange = { host = it },
            label = { Text("电脑 IP 地址") },
            supportingText = { Text("USB(ADB) 连接时填 127.0.0.1") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = port,
            onValueChange = { port = it.filter { c -> c.isDigit() } },
            label = { Text("端口（默认 48000，两端保持一致）") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Spacer(Modifier.height(8.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(
                checked = testSource,
                onCheckedChange = {
                    testSource = it
                    Settings.testSource = it
                },
            )
            Text("测试源模式（不发系统声音，发送内置测试音，用于链路自测）", style = MaterialTheme.typography.bodySmall)
        }
        Spacer(Modifier.height(4.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = {
                    Settings.host = host.trim()
                    Settings.port = port.toIntOrNull() ?: 48000
                    onStart()
                },
                enabled = !state.running,
            ) { Text(if (state.connected) "正在发送…" else "开始发送") }
            Spacer(Modifier.width(12.dp))
            OutlinedButton(
                onClick = onStop,
                enabled = state.running,
                colors = ButtonDefaults.outlinedButtonColors(contentColor = Color(0xFFB3261E)),
            ) { Text("停止") }
        }

        Spacer(Modifier.height(16.dp))
        StatusCard(
            running = state.running,
            connected = state.connected,
            peer = state.peer,
            error = state.error,
            extra = if (state.connected) "已发送 ${formatBytes(state.sentBytes)}（${state.sentSeconds} 秒）" else "",
        )

        Spacer(Modifier.height(8.dp))
        var selfTest by remember { mutableStateOf(false) }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(
                checked = selfTest,
                onCheckedChange = {
                    selfTest = it
                    if (it) dev.audiobridge.app.util.SelfTestTone.start()
                    else dev.audiobridge.app.util.SelfTestTone.stop()
                },
            )
            Text("播放测试音（自测链路）", style = MaterialTheme.typography.bodySmall)
        }

        Spacer(Modifier.height(12.dp))
        Text(
            "使用说明：\n" +
                "1. 电脑端 AudioBridge 选择「接收」并启动监听；\n" +
                "2. 点「扫描局域网设备」，在列表中点「连接」；USB 场景请先用电脑端「USB(ADB)」按钮，再手动填 127.0.0.1；\n" +
                "3. 正常模式首次发送需在系统弹窗中允许录制音频。",
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun ReceiverPane(onStart: () -> Unit, onStop: () -> Unit) {
    val state by AppBus.receiver.collectAsState()
    var port by remember { mutableStateOf(Settings.listenPort.toString()) }
    var jitterMs by remember { mutableStateOf(Settings.jitterMs.toString()) }
    var volume by remember { mutableStateOf(1.0f) }

    // 每 500ms 拉取一次统计
    LaunchedEffect(Unit) {
        while (true) {
            delay(500)
            val svc = PlaybackService.instance
            if (svc != null) {
                AppBus.updateReceiver {
                    it.copy(bufferedMs = svc.bufferedMsSnapshot(), underruns = svc.underrunsSnapshot())
                }
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
    ) {
        Text("电脑的系统声音会通过局域网传到本机，用本机扬声器/耳机播放。", style = MaterialTheme.typography.bodyMedium)

        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = port,
            onValueChange = { port = it.filter { c -> c.isDigit() } },
            label = { Text("监听端口") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = jitterMs,
            onValueChange = { jitterMs = it.filter { c -> c.isDigit() } },
            label = { Text("缓冲水位 ms（默认 50；卡顿调大，求快调小）") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Spacer(Modifier.height(12.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = {
                    Settings.listenPort = port.toIntOrNull() ?: 48000
                    Settings.jitterMs = (jitterMs.toIntOrNull() ?: 50).coerceIn(20, 300)
                    onStart()
                },
                enabled = !state.running,
            ) { Text(if (state.running) "监听中…" else "开始接收") }
            Spacer(Modifier.width(12.dp))
            OutlinedButton(
                onClick = onStop,
                enabled = state.running,
                colors = ButtonDefaults.outlinedButtonColors(contentColor = Color(0xFFB3261E)),
            ) { Text("停止") }
        }

        Spacer(Modifier.height(16.dp))
        Text("音量：${(volume * 100).toInt()}%")
        Slider(value = volume, onValueChange = {
            volume = it
            PlaybackService.instance?.volume = it
        })

        Spacer(Modifier.height(8.dp))
        StatusCard(
            running = state.running,
            connected = state.clientConnected,
            peer = state.peer,
            error = state.error,
            extra = if (state.clientConnected) "缓冲 ${state.bufferedMs}ms，欠载 ${state.underruns} 次" else "",
        )
    }
}

@Composable
private fun StatusCard(
    running: Boolean,
    connected: Boolean,
    peer: String,
    error: String?,
    extra: String,
) {
    val color = when {
        error != null -> Color(0xFFB3261E)
        connected -> Color(0xFF1B873B)
        running -> Color(0xFF8A6D00)
        else -> MaterialTheme.colorScheme.outline
    }
    val text = when {
        error != null -> "错误：$error"
        connected -> "已连接 $peer" + (if (extra.isNotEmpty()) "，$extra" else "")
        running -> "等待连接…" + (if (extra.isNotEmpty()) "，$extra" else "")
        else -> "未启动"
    }
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text("状态：$text", color = color, style = MaterialTheme.typography.bodyMedium)
        }
    }
}

private fun formatBytes(b: Long): String = when {
    b >= 1 shl 20 -> "%.1f MB".format(b / 1048576.0)
    b >= 1 shl 10 -> "%.1f KB".format(b / 1024.0)
    else -> "$b B"
}
