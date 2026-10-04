package dev.audiobridge.app.util

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import kotlin.concurrent.thread
import kotlin.math.PI
import kotlin.math.sin

/**
 * 自测用测试音：以 USAGE_MEDIA 持续播放 440Hz 正弦波。
 *
 * 用途：验证「发送到电脑」链路——系统声音回采会把本 App 播放的该音一并
 * 采走，PC 接收端看到 RMS>0 即代表整条采集链路工作正常。
 */
object SelfTestTone {
    private const val SAMPLE_RATE = 48_000
    private const val FREQ_HZ = 440.0

    @Volatile
    private var playing = false

    private var thread: Thread? = null

    val isPlaying: Boolean
        get() = playing

    @Synchronized
    fun start() {
        if (playing) return
        playing = true
        val minBuf = AudioTrack.getMinBufferSize(
            SAMPLE_RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT,
        )
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                    .build(),
            )
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(maxOf(minBuf, 8192))
            .build()
        thread = thread(name = "ab-selftest-tone") {
            track.play()
            val frames = SAMPLE_RATE / 50 // 20ms
            val buf = ShortArray(frames * 2)
            var phase = 0L
            while (playing) {
                for (i in 0 until frames) {
                    val v = sin(2.0 * PI * FREQ_HZ * phase / SAMPLE_RATE) * 0.3
                    val s = (v * Short.MAX_VALUE).toInt().toShort()
                    buf[i * 2] = s
                    buf[i * 2 + 1] = s
                    phase++
                }
                track.write(buf, 0, buf.size)
            }
            runCatching { track.stop() }
            runCatching { track.release() }
        }
    }

    @Synchronized
    fun stop() {
        playing = false
        thread = null
    }
}
