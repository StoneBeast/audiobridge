package dev.audiobridge.app.play

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import dev.audiobridge.app.AppBus
import dev.audiobridge.app.R
import dev.audiobridge.app.Settings
import dev.audiobridge.app.proto.AckStatus
import dev.audiobridge.app.proto.DataPayload
import dev.audiobridge.app.proto.HelloAck
import dev.audiobridge.app.proto.JitterBuffer
import dev.audiobridge.app.proto.Protocol
import dev.audiobridge.app.proto.parseByePayload
import dev.audiobridge.app.proto.parsePingPayload
import dev.audiobridge.app.proto.pingPayload
import dev.audiobridge.app.proto.readHello
import dev.audiobridge.app.proto.readMessage
import dev.audiobridge.app.proto.sendHelloAck
import dev.audiobridge.app.proto.sendMessage
import java.net.InetSocketAddress
import java.net.ServerSocket
import kotlin.concurrent.thread

/**
 * 接收端前台服务：监听 TCP，接收 DATA 帧写入抖动缓冲，
 * AudioTrack 周期性从缓冲取数播放（不足补静音）。
 */
class PlaybackService : Service() {

    companion object {
        private const val NOTIFICATION_ID = 102
        private const val CHANNEL_ID = "audiobridge_playback"
        private const val TAG = "ab-play"

        @Volatile
        var instance: PlaybackService? = null
            private set

        fun start(context: Context) {
            context.startForegroundService(Intent(context, PlaybackService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, PlaybackService::class.java))
        }
    }

    // 抖动缓冲的目标水位来自用户设置，需在 onCreate（有 Context）后创建
    private lateinit var jitter: JitterBuffer
    private var server: ServerSocket? = null
    private var listenThread: Thread? = null
    private var playerThread: Thread? = null
    private var track: AudioTrack? = null

    @Volatile
    private var active = false

    @Volatile
    private var clientActive = false

    private val discoveryStop = java.util.concurrent.atomic.AtomicBoolean(false)

    @Volatile
    var volume: Float = 1.0f

    /** UI 轮询用：当前缓冲水位（毫秒）。 */
    fun bufferedMsSnapshot(): Long = if (::jitter.isInitialized) jitter.bufferedMs() else 0

    /** UI 轮询用：累计欠载次数。 */
    fun underrunsSnapshot(): Long = if (::jitter.isInitialized) jitter.stats().underruns else 0

    override fun onCreate() {
        super.onCreate()
        jitter = JitterBuffer(
            Protocol.SAMPLE_RATE, Protocol.CHANNELS,
            Protocol.DEFAULT_FRAME_MS, Settings.jitterMs, 250,
        )
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (active) return START_STICKY
        startAsForeground()
        instance = this
        active = true
        val port = Settings.listenPort
        startPlayerThread()
        startListenThread(port)
        // 局域网自动发现：应答 UDP 探测（与 TCP 监听同端口）
        discoveryStop.set(false)
        dev.audiobridge.app.proto.Discovery.spawnResponder(
            port, port, deviceName(), discoveryStop,
        )
        AppBus.updateReceiver { it.copy(running = true, error = null, peer = "", clientConnected = false) }
        return START_STICKY
    }

    override fun onDestroy() {
        active = false
        instance = null
        discoveryStop.set(true)
        runCatching { server?.close() }
        runCatching { track?.stop() }
        runCatching { track?.release() }
        track = null
        AppBus.updateReceiver { it.copy(running = false, clientConnected = false) }
        super.onDestroy()
    }

    private fun startAsForeground() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "AudioBridge 接收", NotificationManager.IMPORTANCE_LOW),
        )
        val notif: Notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_send_audio)
            .setContentTitle("AudioBridge 正在接收音频")
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 30) {
            startForeground(NOTIFICATION_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(NOTIFICATION_ID, notif)
        }
    }

    private fun startPlayerThread() {
        val chunk = Protocol.s16BytesPerMs(Protocol.SAMPLE_RATE, 2) * Protocol.DEFAULT_FRAME_MS
        val minBuf = AudioTrack.getMinBufferSize(
            Protocol.SAMPLE_RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT,
        )
        val t = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(Protocol.SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                    .build(),
            )
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(maxOf(minBuf, chunk * 8))
            .build()
        track = t
        t.play()
        val out = ByteArray(chunk)
        playerThread = thread(name = "ab-player") {
            try {
                while (active) {
                    jitter.pop(out)
                    applyVolume(out)
                    val written = t.write(out, 0, out.size, AudioTrack.WRITE_BLOCKING)
                    if (written < 0) break
                }
            } finally {
                runCatching { t.stop() }
                runCatching { t.release() }
            }
        }
    }

    private fun applyVolume(buf: ByteArray) {
        val v = volume
        if (v >= 0.999f) return
        var i = 0
        while (i < buf.size) {
            val lo = buf[i].toInt() and 0xFF
            val hi = buf[i + 1].toInt()
            var sample = ((hi shl 8) or lo).toShort().toInt()
            sample = (sample * v).toInt().coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt())
            buf[i] = (sample and 0xFF).toByte()
            buf[i + 1] = ((sample shr 8) and 0xFF).toByte()
            i += 2
        }
    }

    private fun startListenThread(port: Int) {
        listenThread = thread(name = "ab-listen") {
            while (active) {
                val ss = ServerSocket()
                try {
                    ss.reuseAddress = true
                    ss.bind(InetSocketAddress(port))
                    server = ss
                    Log.i(TAG, "listening on :$port")
                    while (active) {
                        val s = ss.accept()
                        handleClient(s)
                    }
                } catch (e: Exception) {
                    if (active) {
                        Log.w(TAG, "accept loop: ${e.message}; retry in 1s")
                        AppBus.updateReceiver { it.copy(error = "监听失败: ${e.message}，1 秒后重试") }
                        Thread.sleep(1000)
                    }
                } finally {
                    runCatching { ss.close() }
                }
            }
        }
    }

    private fun handleClient(s: java.net.Socket) {
        // 同一时刻只接受一路流
        if (clientActive) {
            runCatching {
                s.tcpNoDelay = true
                sendHelloAck(
                    s.getOutputStream(),
                    HelloAck(AckStatus.BUSY, Protocol.CODEC_PCM_S16LE, Protocol.SAMPLE_RATE, Protocol.CHANNELS, deviceName(), 80),
                )
                s.close()
            }
            return
        }
        val t = thread(name = "ab-conn") {
            clientActive = true
            try {
                s.tcpNoDelay = true
                val input = s.getInputStream()
                val out = s.getOutputStream()
                val hello = readHello(input)
                val ack = HelloAck(
                    status = AckStatus.OK,
                    codec = Protocol.CODEC_PCM_S16LE,
                    sampleRate = Protocol.SAMPLE_RATE,
                    channels = Protocol.CHANNELS,
                    deviceName = deviceName(),
                    jitterTargetMs = Settings.jitterMs,
                )
                sendHelloAck(out, ack)
                jitter.reset()
                AppBus.updateReceiver {
                    it.copy(clientConnected = true, peer = "${s.inetAddress?.hostAddress}:${s.port}（${hello.deviceName}）", error = null)
                }
                Log.i(TAG, "client connected: $hello")

                while (active) {
                    val msg = readMessage(input)
                    when (msg.type) {
                        Protocol.MSG_DATA -> {
                            val dp = DataPayload.fromBytes(msg.payload)
                            jitter.push(dp.audio)
                        }
                        Protocol.MSG_PING -> sendMessage(out, Protocol.MSG_PONG, msg.payload)
                        Protocol.MSG_PONG -> Unit
                        Protocol.MSG_BYE -> {
                            val (reason, note) = parseByePayload(msg.payload)
                            Log.i(TAG, "peer said bye: $reason $note")
                            break
                        }
                        else -> Log.w(TAG, "unknown msg type ${msg.type}")
                    }
                }
            } catch (e: Exception) {
                if (active) Log.i(TAG, "connection ended: ${e.message}")
            } finally {
                clientActive = false
                jitter.reset()
                runCatching { s.close() }
                AppBus.updateReceiver { it.copy(clientConnected = false, peer = "") }
            }
        }
        t.isDaemon = false
    }

    private fun deviceName(): String = "${Build.MANUFACTURER} ${Build.MODEL}".trim().ifEmpty { "Android" }
}
