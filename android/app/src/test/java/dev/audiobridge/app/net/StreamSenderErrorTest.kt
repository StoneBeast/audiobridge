package dev.audiobridge.app.net

import dev.audiobridge.app.proto.Hello
import dev.audiobridge.app.proto.Protocol
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException

/**
 * 连接失败时的错误提示测试。
 *
 * 背景：Android 的 connect 超时异常没有 message，直接 `${e.message}` 会显示
 * 成「启动失败: null」。StreamSender.connect 必须把它翻译成可操作的提示。
 */
class StreamSenderErrorTest {

    private fun hello() = Hello(
        deviceName = "JUnit",
        codec = Protocol.CODEC_PCM_S16LE,
        sampleRate = Protocol.SAMPLE_RATE,
        channels = Protocol.CHANNELS,
        frameMs = Protocol.DEFAULT_FRAME_MS,
        bufferTargetMs = Protocol.DEFAULT_TARGET_MS,
        authTokenSha256 = ByteArray(32),
    )

    private fun closedLocalPort(): Int =
        java.net.ServerSocket(0).let { s ->
            val p = s.localPort
            s.close()
            p
        }

    @Test
    fun refusedConnectionGetsActionableHint() {
        val port = closedLocalPort()
        val sender = StreamSender("127.0.0.1", port, hello())
        try {
            sender.connect()
            error("connect should have failed")
        } catch (e: IOException) {
            val msg = e.message ?: ""
            assertTrue("missing 无法连接 prefix: $msg", msg.contains("无法连接"))
            assertTrue("missing refused hint: $msg", msg.contains("连接被拒绝"))
            assertTrue("missing target addr: $msg", msg.contains("127.0.0.1:$port"))
            assertFalse("bare null must not appear: $msg", msg.contains("null"))
        } finally {
            sender.close()
        }
    }

    @Test
    fun timeoutErrorGetsActionableHint() {
        // 不可路由的测试网段（RFC 5737 TEST-NET-1），预期连接超时
        val sender = StreamSender("192.0.2.1", 48000, hello())
        try {
            sender.connect()
            error("connect should have failed")
        } catch (e: IOException) {
            val msg = e.message ?: ""
            assertTrue(
                "timeout should map to actionable hint, got: $msg",
                msg.contains("连接超时") || msg.contains("连接被拒绝") || msg.contains("网络不可达") ||
                    msg.contains("无法连接"),
            )
            assertFalse("bare null must not appear: $msg", msg.contains("null"))
        } finally {
            sender.close()
        }
    }
}
