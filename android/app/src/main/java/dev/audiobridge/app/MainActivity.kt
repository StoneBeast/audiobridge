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
                        onStartSend = { requestProjection() },
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
    var tab by remember { mutableIntStateOf(0) }
    Column(modifier = Modifier.fillMaxSize()) {
        Text(
            text = "AudioBridge 音频桥",
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(16.dp),
        )
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
private fun SenderPane(onStart: () -> Unit, onStop: () -> Unit) {
    val state by AppBus.sender.collectAsState()
    var host by remember { mutableStateOf(Settings.host) }
    var port by remember { mutableStateOf(Settings.port.toString()) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
    ) {
        Text("把本机的系统声音（电视剧、音乐等）实时发送到局域网内的电脑。", style = MaterialTheme.typography.bodyMedium)

        Spacer(Modifier.height(12.dp))
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
            label = { Text("端口") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Spacer(Modifier.height(12.dp))
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

        Spacer(Modifier.height(16.dp))
        Text(
            "使用说明：\n" +
                "1. 电脑端 AudioBridge 选择「接收」并启动监听；\n" +
                "2. 局域网：填电脑 IP；USB：先用电脑端「USB(ADB)」按钮建立隧道，然后这里填 127.0.0.1；\n" +
                "3. 点「开始发送」，在系统弹窗中允许录制音频。",
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun ReceiverPane(onStart: () -> Unit, onStop: () -> Unit) {
    val state by AppBus.receiver.collectAsState()
    var port by remember { mutableStateOf(Settings.listenPort.toString()) }
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
        Spacer(Modifier.height(12.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = {
                    Settings.listenPort = port.toIntOrNull() ?: 48000
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
