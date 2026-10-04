package dev.audiobridge.app.capture

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.IntentCompat
import dev.audiobridge.app.AppBus
import dev.audiobridge.app.R
import dev.audiobridge.app.Settings
import dev.audiobridge.app.net.StreamSender
import dev.audiobridge.app.proto.Hello
import dev.audiobridge.app.proto.Protocol
import dev.audiobridge.app.util.sha256Token
import kotlin.concurrent.thread

/**
 * 发送端前台服务：MediaProjection 系统音频捕获 + TCP 推流。
 *
 * API 34 要求：必须先以前台服务（mediaProjection 类型）身份运行，
 * 才能 getMediaProjection()——因此采集授权结果通过 Intent 传给本服务，
 * 在 startForeground() 之后才申请 MediaProjection。
 */
class CaptureService : Service() {

    companion object {
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_DATA = "data"
        const val ACTION_TEST = "dev.audiobridge.app.action.TEST_SEND"
        private const val NOTIFICATION_ID = 101
        private const val CHANNEL_ID = "audiobridge_capture"
        private const val TAG = "ab-capture"

        /** 正常模式：携带 MediaProjection 授权结果启动。 */
        fun start(context: Context, resultCode: Int, data: Intent) {
            val intent = Intent(context, CaptureService::class.java)
                .putExtra(EXTRA_RESULT_CODE, resultCode)
                .putExtra(EXTRA_DATA, data)
            context.startForegroundService(intent)
        }

        /** 测试源模式：跳过系统采集，发送内置测试音。 */
        fun startTest(context: Context) {
            val intent = Intent(context, CaptureService::class.java).setAction(ACTION_TEST)
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, CaptureService::class.java))
        }
    }

    private val mainHandler = Handler(Looper.getMainLooper())
    private var projection: MediaProjection? = null
    private var record: AudioRecord? = null
    private var sender: StreamSender? = null
    private var captureThread: Thread? = null

    @Volatile
    private var active = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val isTest = intent?.action == ACTION_TEST
        if (isTest) {
            startAsForeground(testSource = true)
            try {
                startTestCapture()
            } catch (e: Exception) {
                Log.e(TAG, "start test capture failed", e)
                AppBus.updateSender { it.copy(running = false, error = "启动失败: ${e.message}") }
                stopCapture()
                stopSelf()
            }
            return START_NOT_STICKY
        }
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Int.MIN_VALUE) ?: Int.MIN_VALUE
        val data = intent?.let {
            IntentCompat.getParcelableExtra(it, EXTRA_DATA, Intent::class.java)
        }
        if (resultCode == Int.MIN_VALUE || data == null) {
            AppBus.updateSender { it.copy(running = false, error = "缺少采集授权参数") }
            stopSelf()
            return START_NOT_STICKY
        }
        startAsForeground(testSource = false)
        try {
            startCapture(resultCode, data)
        } catch (e: Exception) {
            Log.e(TAG, "start capture failed", e)
            AppBus.updateSender { it.copy(running = false, error = "启动失败: ${e.message}") }
            stopCapture()
            stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        stopCapture()
        super.onDestroy()
    }

    private fun startAsForeground(testSource: Boolean) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "AudioBridge 发送", NotificationManager.IMPORTANCE_LOW),
        )
        val title = if (testSource) "AudioBridge 正在发送测试音（自测模式）" else "AudioBridge 正在发送系统声音"
        val notif: Notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_send_audio)
            .setContentTitle(title)
            .setContentText("点击通知栏可停止")
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 30) {
            val type = if (testSource) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
            } else {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            }
            startForeground(NOTIFICATION_ID, notif, type)
        } else {
            startForeground(NOTIFICATION_ID, notif)
        }
    }

    private fun hello(): Hello {
        val token = Settings.token
        return Hello(
            deviceName = "${Build.MANUFACTURER} ${Build.MODEL}".trim().ifEmpty { "Android" },
            codec = Protocol.CODEC_PCM_S16LE,
            sampleRate = Protocol.SAMPLE_RATE,
            channels = Protocol.CHANNELS,
            frameMs = Protocol.DEFAULT_FRAME_MS,
            bufferTargetMs = Protocol.DEFAULT_TARGET_MS,
            authTokenSha256 = sha256Token(token),
        )
    }

    /** 测试源模式：以内置测试音为音频源（不需要 MediaProjection）。 */
    private fun startTestCapture() {
        val host = Settings.host
        val port = Settings.port
        val s = StreamSender(host, port, hello())
        s.connect()
        sender = s

        val chunkBytes = Protocol.s16BytesPerMs(Protocol.SAMPLE_RATE, 2) * Protocol.DEFAULT_FRAME_MS
        val tone = dev.audiobridge.app.util.TestToneSource(Protocol.DEFAULT_FRAME_MS)
        val buf = ByteArray(chunkBytes)
        active = true
        AppBus.updateSender {
            it.copy(running = true, connected = true, peer = "$host:$port", error = null, sentBytes = 0, sentSeconds = 0)
        }
        Log.i(TAG, "test-source streaming -> $host:$port")
        captureThread = thread(name = "ab-capture-test") {
            var sent = 0L
            val startedAt = System.currentTimeMillis()
            try {
                while (active) {
                    val n = tone.read(buf)
                    if (n > 0) {
                        s.sendAudio(buf, 0, n)
                        sent += n
                        if (sent % (chunkBytes * 25L) == 0L) {
                            val sec = (System.currentTimeMillis() - startedAt) / 1000
                            AppBus.updateSender { it.copy(sentBytes = sent, sentSeconds = sec) }
                        }
                    }
                }
            } catch (e: Exception) {
                if (active) {
                    Log.e(TAG, "send loop error", e)
                    AppBus.updateSender { it.copy(error = "发送中断: ${e.message}", connected = false) }
                }
            } finally {
                active = false
                runCatching { s.close() }
                AppBus.updateSender { it.copy(running = false, connected = false) }
                mainHandler.post { stopSelf() }
            }
        }
    }

    private fun startCapture(resultCode: Int, data: Intent) {
        val sm = getSystemService(MediaProjectionManager::class.java)
        val mp = sm.getMediaProjection(resultCode, data)
        projection = mp
        mp.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                Log.i(TAG, "media projection stopped by system/user")
                mainHandler.post {
                    stopCapture()
                    stopSelf()
                }
            }
        }, mainHandler)

        val captureConfig = AudioPlaybackCaptureConfiguration.Builder(mp)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()

        val audioFormat = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(Protocol.SAMPLE_RATE)
            .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
            .build()

        val minBuf = AudioRecord.getMinBufferSize(
            Protocol.SAMPLE_RATE, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT,
        )
        val bufSize = maxOf(minBuf, Protocol.s16BytesPerMs(Protocol.SAMPLE_RATE, 2) * 100)

        val rec = AudioRecord.Builder()
            .setAudioFormat(audioFormat)
            .setBufferSizeInBytes(bufSize)
            .setAudioPlaybackCaptureConfig(captureConfig)
            .build()
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            throw IllegalStateException("AudioRecord 初始化失败（系统音频捕获不可用）")
        }
        record = rec

        val host = Settings.host
        val port = Settings.port
        val s = StreamSender(host, port, hello())
        s.connect() // 失败会抛出，交由 onStartCommand 兜底
        sender = s

        val chunkBytes = Protocol.s16BytesPerMs(Protocol.SAMPLE_RATE, 2) * Protocol.DEFAULT_FRAME_MS
        val buf = ByteArray(chunkBytes)
        rec.startRecording()
        active = true
        AppBus.updateSender {
            it.copy(running = true, connected = true, peer = "$host:$port", error = null, sentBytes = 0, sentSeconds = 0)
        }
        Log.i(TAG, "capturing -> $host:$port (chunk=$chunkBytes bytes)")

        captureThread = thread(name = "ab-capture") {
            var sent = 0L
            val startedAt = System.currentTimeMillis()
            try {
                while (active) {
                    val n = rec.read(buf, 0, buf.size)
                    if (n > 0) {
                        s.sendAudio(buf, 0, n)
                        sent += n
                        if (sent % (chunkBytes * 25L) == 0L) { // ~每秒更新一次
                            val sec = (System.currentTimeMillis() - startedAt) / 1000
                            AppBus.updateSender { it.copy(sentBytes = sent, sentSeconds = sec) }
                        }
                    } else if (n < 0) {
                        break
                    }
                }
            } catch (e: Exception) {
                if (active) {
                    Log.e(TAG, "send loop error", e)
                    AppBus.updateSender { it.copy(error = "发送中断: ${e.message}", connected = false) }
                }
            } finally {
                active = false
                runCatching { rec.stop() }
                runCatching { s.close() }
                AppBus.updateSender { it.copy(running = false, connected = false) }
                mainHandler.post { stopSelf() }
            }
        }
    }

    private fun stopCapture() {
        active = false
        runCatching { sender?.close() }
        runCatching { record?.stop() }
        runCatching { record?.release() }
        runCatching { projection?.stop() }
        sender = null
        record = null
        projection = null
        AppBus.updateSender { it.copy(running = false, connected = false) }
    }
}
