package dev.audiobridge.app.util

import dev.audiobridge.app.proto.Protocol
import kotlin.concurrent.thread
import kotlin.math.PI
import kotlin.math.sin

/**
 * 内置测试音数据源（不做采集，直接生成 440Hz 正弦波 S16 立体声）。
 *
 * 用途：「测试源模式」——当设备 ROM 不支持系统声音回采（部分模拟器）或
 * 需要单独验证网络链路时，用生成的音频替代 AudioRecord。
 * 与 audiobridge-core 的 ToneSource 行为一致（自带实时节拍）。
 */
class TestToneSource(
    private val frameMs: Int = 20,
    private val freqHz: Double = 440.0,
    private val amplitude: Double = 0.3,
) {
    private val chunkBytes =
        Protocol.s16BytesPerMs(Protocol.SAMPLE_RATE, Protocol.CHANNELS) * frameMs.coerceAtLeast(1)
    private var phase = 0L

    /** 填充一帧音频到 buf（需 ≥ chunkBytes），返回字节数；自带节拍。 */
    fun read(buf: ByteArray): Int {
        val n = chunkBytes.coerceAtMost(buf.size)
        val frames = n / 4
        for (i in 0 until frames) {
            val v = sin(2.0 * PI * freqHz * phase / Protocol.SAMPLE_RATE) * amplitude
            val s = (v * Short.MAX_VALUE).toInt().toShort()
            val lo = (s.toInt() and 0xFF).toByte()
            val hi = ((s.toInt() shr 8) and 0xFF).toByte()
            buf[i * 4] = lo
            buf[i * 4 + 1] = hi
            buf[i * 4 + 2] = lo
            buf[i * 4 + 3] = hi
            phase++
        }
        Thread.sleep(frameMs.toLong().coerceAtLeast(1))
        return n
    }
}
