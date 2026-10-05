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

    /** 连接并完成握手；失败抛带可读原因的 IOException。 */
    fun connect() {
        val s = Socket()
        s.tcpNoDelay = true
        try {
            s.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
        } catch (e: Exception) {
            runCatching { s.close() }
            // Android 的 connect 超时异常没有 message（显示为 null），这里翻译成可操作提示
            val hint = when (e) {
                is java.net.SocketTimeoutException ->
                    "连接超时（${CONNECT_TIMEOUT_MS / 1000} 秒无响应）——请检查：" +
                        "① 电脑端是否已点「开始接收」；② Windows 防火墙是否放行 AudioBridge " +
                        "（首次监听时的弹窗要点「允许」，若错过请到防火墙设置里放行 48000 端口）；" +
                        "③ 两台设备是否在同一网络（或改用 USB 隧道）"
                is java.net.ConnectException ->
                    "连接被拒绝——电脑端 AudioBridge 是否已启动接收？防火墙是否拦截？"
                is java.net.UnknownHostException ->
                    "地址无法解析——请检查 IP 是否填写正确"
                else -> "${e.javaClass.simpleName}: ${e.message ?: "(无详细信息)"}"
            }
            throw java.io.IOException("无法连接 $host:$port —— $hint", e)
        }
        s.soTimeout = CONNECT_TIMEOUT_MS // 仅握手读有超时
        socket = s
        out = s.getOutputStream()
        input = s.getInputStream()
        ack = try {
            handshake(out, input, hello)
        } catch (e: Exception) {
            runCatching { s.close() }
            socket = null
            throw java.io.IOException(
                "已连上 $host:$port 但握手失败——${e.javaClass.simpleName}: ${e.message ?: "(无详细信息)"}" +
                    "（对端可能不是 AudioBridge 接收端，或访问令牌不一致）",
                e,
            )
        }
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
