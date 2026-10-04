package dev.audiobridge.app.net

import android.util.Log
import dev.audiobridge.app.proto.DataPayload
import dev.audiobridge.app.proto.Hello
import dev.audiobridge.app.proto.HelloAck
import dev.audiobridge.app.proto.Protocol
import dev.audiobridge.app.proto.byePayload
import dev.audiobridge.app.proto.handshake
import dev.audiobridge.app.proto.sendMessage
import java.io.Closeable
import java.io.InputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket

/** 发送端连接封装：TCP 连接 + 握手 + DATA 帧发送。 */
class StreamSender(
    private val host: String,
    private val port: Int,
    private val hello: Hello,
) : Closeable {

    private var socket: Socket? = null
    private lateinit var out: OutputStream
    private lateinit var input: InputStream
    private var seq = 0L
    private val startedAt = monotonicMs()
    var ack: HelloAck? = null
        private set

    /** 连接并完成握手；失败抛异常。 */
    fun connect() {
        val s = Socket()
        s.tcpNoDelay = true
        s.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
        s.soTimeout = CONNECT_TIMEOUT_MS // 仅握手读有超时
        socket = s
        out = s.getOutputStream()
        input = s.getInputStream()
        ack = handshake(out, input, hello)
        s.soTimeout = 0 // 数据阶段不再读对端，取消读超时
        Log.i(TAG, "handshake ok: $ack")
    }

    /** 发送一段音频（采集线程调用；阻塞写提供天然背压）。 */
    fun sendAudio(data: ByteArray, off: Int, len: Int) {
        val payload = DataPayload(
            seq = seq++,
            timestampMs = monotonicMs() - startedAt,
            audio = data.copyOfRange(off, off + len),
        )
        sendMessage(out, Protocol.MSG_DATA, payload.toBytes())
    }

    /** 礼貌断开：发送 BYE 后关闭。 */
    fun sendBye(reason: Int = 1) {
        try {
            sendMessage(out, Protocol.MSG_BYE, byePayload(reason, ""))
        } catch (_: Exception) {
        }
    }

    override fun close() {
        sendBye()
        try {
            socket?.close()
        } catch (_: Exception) {
        }
        socket = null
    }

    companion object {
        private const val TAG = "ab-sender"
        private const val CONNECT_TIMEOUT_MS = 5000

        fun monotonicMs(): Long = System.nanoTime() / 1_000_000L
    }
}
