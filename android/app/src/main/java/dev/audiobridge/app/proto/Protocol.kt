package dev.audiobridge.app.proto

import java.io.IOException
import java.io.OutputStream

/**
 * 线协议常量与握手消息。
 *
 * 字节序一律小端；规范见 docs/protocol.md。
 * 本文件是 audiobridge-core（Rust）协议实现的 Kotlin 镜像，
 * 两侧由 protocol/conformance-vectors.json 做字节级一致性约束。
 */
object Protocol {
    val MAGIC: ByteArray = byteArrayOf(0x41, 0x42, 0x52, 0x47) // "ABRG"
    const val PROTOCOL_VERSION = 1

    const val MSG_HELLO = 1
    const val MSG_HELLO_ACK = 2
    const val MSG_DATA = 3
    const val MSG_PING = 4
    const val MSG_PONG = 5
    const val MSG_BYE = 6

    const val CODEC_PCM_S16LE: Byte = 0
    const val CODEC_OPUS: Byte = 1

    const val SAMPLE_RATE = 48_000
    const val CHANNELS = 2
    const val DEFAULT_FRAME_MS = 20
    const val DEFAULT_TARGET_MS = 80
    const val MAX_PAYLOAD = 65_535
    const val MAX_NAME_LEN = 255

    fun s16BytesPerMs(sampleRate: Int, channels: Int): Int =
        sampleRate * channels * 2 / 1000

    fun msgTypeName(type: Int): String = when (type) {
        MSG_HELLO -> "HELLO"
        MSG_HELLO_ACK -> "HELLO_ACK"
        MSG_DATA -> "DATA"
        MSG_PING -> "PING"
        MSG_PONG -> "PONG"
        MSG_BYE -> "BYE"
        else -> "UNKNOWN($type)"
    }
}

class ProtocolException(message: String) : IOException(message)

/** HELLO_ACK 的 status 字段取值。 */
object AckStatus {
    const val OK = 0
    const val BUSY = 1
    const val VERSION_MISMATCH = 2
    const val FORMAT_UNSUPPORTED = 3
    const val AUTH_FAILED = 4

    fun name(status: Int): String = when (status) {
        OK -> "OK"
        BUSY -> "BUSY"
        VERSION_MISMATCH -> "VERSION_MISMATCH"
        FORMAT_UNSUPPORTED -> "FORMAT_UNSUPPORTED"
        AUTH_FAILED -> "AUTH_FAILED"
        else -> "OTHER($status)"
    }
}

/** 发起方（音频发送端）在握手阶段发出的请求。 */
class Hello(
    val deviceName: String,
    val codec: Byte,
    val sampleRate: Int,
    val channels: Int,
    val frameMs: Int,
    val bufferTargetMs: Int,
    val authTokenSha256: ByteArray,
) {
    fun toBytes(): ByteArray {
        val name = deviceName.toByteArray(Charsets.UTF_8).let {
            if (it.size > Protocol.MAX_NAME_LEN) it.copyOf(Protocol.MAX_NAME_LEN) else it
        }
        return ByteWriter()
            .u16(name.size).bytes(name)
            .u8(codec.toInt() and 0xFF)
            .u32(sampleRate.toLong())
            .u8(channels)
            .u16(frameMs)
            .u16(bufferTargetMs)
            .bytes(authTokenSha256)
            .toByteArray()
    }

    override fun toString(): String =
        "Hello(name=$deviceName, codec=$codec, rate=$sampleRate, ch=$channels, frameMs=$frameMs, target=$bufferTargetMs)"

    companion object {
        fun fromBytes(buf: ByteArray): Hello {
            val r = ByteReader(buf)
            val nameLen = r.u16()
            if (nameLen > Protocol.MAX_NAME_LEN) throw ProtocolException("device name too long: $nameLen")
            val deviceName = r.utf8(nameLen)
            val codec = r.u8().toByte()
            val sampleRate = r.u32().toInt()
            val channels = r.u8()
            val frameMs = r.u16()
            val bufferTargetMs = r.u16()
            val token = r.take(32)
            if (sampleRate == 0 || frameMs == 0) throw ProtocolException("invalid sample_rate/frame_ms")
            if (channels == 0 || channels > 8) throw ProtocolException("invalid channels: $channels")
            return Hello(deviceName, codec, sampleRate, channels, frameMs, bufferTargetMs, token)
        }
    }
}

/** 接收方对 HELLO 的应答。 */
class HelloAck(
    val status: Int,
    val codec: Byte,
    val sampleRate: Int,
    val channels: Int,
    val deviceName: String,
    val jitterTargetMs: Int,
) {
    fun toBytes(): ByteArray {
        val name = deviceName.toByteArray(Charsets.UTF_8).let {
            if (it.size > Protocol.MAX_NAME_LEN) it.copyOf(Protocol.MAX_NAME_LEN) else it
        }
        return ByteWriter()
            .u8(status)
            .u8(codec.toInt() and 0xFF)
            .u32(sampleRate.toLong())
            .u8(channels)
            .u16(name.size).bytes(name)
            .u16(jitterTargetMs)
            .toByteArray()
    }

    override fun toString(): String =
        "HelloAck(status=${AckStatus.name(status)}, name=$deviceName, rate=$sampleRate, ch=$channels, jitter=$jitterTargetMs)"

    companion object {
        fun fromBytes(buf: ByteArray): HelloAck {
            val r = ByteReader(buf)
            val status = r.u8()
            val codec = r.u8().toByte()
            val sampleRate = r.u32().toInt()
            val channels = r.u8()
            val nameLen = r.u16()
            if (nameLen > Protocol.MAX_NAME_LEN) throw ProtocolException("device name too long: $nameLen")
            val deviceName = r.utf8(nameLen)
            val jitterTargetMs = r.u16()
            return HelloAck(status, codec, sampleRate, channels, deviceName, jitterTargetMs)
        }
    }
}

/** DATA 帧负载。 */
class DataPayload(val seq: Long, val timestampMs: Long, val audio: ByteArray) {
    fun toBytes(): ByteArray =
        ByteWriter().u32(seq).u32(timestampMs).bytes(audio).toByteArray()

    override fun toString(): String =
        "DataPayload(seq=$seq, ts=$timestampMs, bytes=${audio.size})"

    companion object {
        fun fromBytes(buf: ByteArray): DataPayload {
            if (buf.size < 8) throw ProtocolException("data payload truncated: ${buf.size} < 8")
            val r = ByteReader(buf)
            val seq = r.u32()
            val timestampMs = r.u32()
            val audio = r.take(buf.size - 8)
            return DataPayload(seq, timestampMs, audio)
        }
    }
}

/** 小端字节写出器。 */
internal class ByteWriter {
    private val out = java.io.ByteArrayOutputStream()

    fun u8(v: Int): ByteWriter { out.write(v and 0xFF); return this }

    fun u16(v: Int): ByteWriter {
        out.write(v and 0xFF)
        out.write((v shr 8) and 0xFF)
        return this
    }

    fun u32(v: Long): ByteWriter {
        for (i in 0..3) out.write(((v shr (8 * i)) and 0xFF).toInt())
        return this
    }

    fun bytes(b: ByteArray): ByteWriter { out.write(b); return this }

    fun toByteArray(): ByteArray = out.toByteArray()
}

/** 小端字节读取游标，带截断检测。 */
internal class ByteReader(val buf: ByteArray) {
    var pos = 0
        private set

    fun take(n: Int): ByteArray {
        if (n < 0 || n > buf.size - pos) {
            throw ProtocolException("truncated: need $n bytes at offset $pos, buffer size ${buf.size}")
        }
        val out = buf.copyOfRange(pos, pos + n)
        pos += n
        return out
    }

    fun u8(): Int = take(1)[0].toInt() and 0xFF

    fun u16(): Int {
        val b = take(2)
        return (b[0].toInt() and 0xFF) or ((b[1].toInt() and 0xFF) shl 8)
    }

    fun u32(): Long {
        val b = take(4)
        var v = 0L
        for (i in 3 downTo 0) v = (v shl 8) or (b[i].toLong() and 0xFF)
        return v
    }

    fun utf8(n: Int): String = String(take(n), Charsets.UTF_8)
}
