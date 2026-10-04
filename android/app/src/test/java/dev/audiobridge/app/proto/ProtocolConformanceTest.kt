package dev.audiobridge.app.proto

import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

/**
 * 与 Rust 侧共享的一致性测试：protocol/conformance-vectors.json
 * 由 audiobridge-core 的 conformance 测试生成，本测试做双向校验：
 * 字节 -> 字段（解码），字段 -> 字节（编码）。
 * 任何一侧破坏线协议兼容性都会在此失败。
 */
class ProtocolConformanceTest {

    private val vectors: JSONObject by lazy {
        val s = javaClass.getResourceAsStream("/conformance-vectors.json")
            ?: error("conformance-vectors.json 不在测试资源路径中（检查 app/build.gradle.kts 的 testOptions sourceSets）")
        JSONObject(s.readBytes().toString(Charsets.UTF_8))
    }

    private fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

    private fun unhex(s: String): ByteArray =
        ByteArray(s.length / 2) { i ->
            ((Character.digit(s[i * 2], 16) shl 4) + Character.digit(s[i * 2 + 1], 16)).toByte()
        }

    @Test
    fun allCasesDecodeAndReEncodeIdentically() {
        val cases = vectors.getJSONArray("cases")
        var matched = 0
        for (i in 0 until cases.length()) {
            val c = cases.getJSONObject(i)
            val bytes = unhex(c.getString("bytes"))
            val fields = c.getJSONObject("fields")
            when (c.getString("kind")) {
                "hello" -> {
                    val h = Hello.fromBytes(bytes)
                    assertEquals(fields.getString("device_name"), h.deviceName)
                    assertEquals(Protocol.CODEC_PCM_S16LE, h.codec)
                    assertEquals(fields.getInt("sample_rate"), h.sampleRate)
                    assertEquals(fields.getInt("channels"), h.channels)
                    assertEquals(fields.getInt("frame_ms"), h.frameMs)
                    assertEquals(fields.getInt("buffer_target_ms"), h.bufferTargetMs)
                    val token = fields.optString("auth_token", "")
                    if (token.isEmpty()) {
                        assertTrue(h.authTokenSha256.all { it == 0.toByte() })
                    } else {
                        assertEquals(token, hex(h.authTokenSha256))
                    }
                    assertArrayEquals("hello encode mismatch", bytes, h.toBytes())
                    matched++
                }
                "hello_ack" -> {
                    val a = HelloAck.fromBytes(bytes)
                    val expectStatus = fields.getString("status")
                    val expect = mapOf(
                        "ok" to AckStatus.OK,
                        "busy" to AckStatus.BUSY,
                    )
                    assertEquals(expect[expectStatus], a.status)
                    if (expectStatus == "ok") {
                        assertEquals(fields.getString("device_name"), a.deviceName)
                        assertEquals(fields.getInt("jitter_target_ms"), a.jitterTargetMs)
                        assertEquals(fields.getInt("sample_rate"), a.sampleRate)
                        assertEquals(fields.getInt("channels"), a.channels)
                    }
                    assertArrayEquals("hello_ack encode mismatch", bytes, a.toBytes())
                    matched++
                }
                "data_payload" -> {
                    val d = DataPayload.fromBytes(bytes)
                    assertEquals(fields.getLong("seq"), d.seq)
                    assertEquals(fields.getLong("timestamp_ms"), d.timestampMs)
                    assertEquals(fields.getInt("audio_len"), d.audio.size)
                    assertArrayEquals("data encode mismatch", bytes, d.toBytes())
                    matched++
                }
                "message" -> {
                    val msg = readMessage(ByteArrayInputStream(bytes))
                    assertEquals(fields.getInt("msg_type"), msg.type)
                    assertEquals(fields.getInt("payload_len"), msg.payload.size)
                    if (msg.type == Protocol.MSG_DATA) {
                        DataPayload.fromBytes(msg.payload)
                    }
                    // 重新编码应逐字节一致
                    val out = ByteArrayOutputStream()
                    sendMessage(out, msg.type, msg.payload)
                    assertArrayEquals("message encode mismatch", bytes, out.toByteArray())
                    matched++
                }
                "discovery_probe" -> {
                    val parsed = Discovery.parseProbe(bytes, bytes.size)
                    assertTrue("probe parse failed", parsed)
                    // 编码方向：重新构造探测包应逐字节一致
                    assertArrayEquals("probe encode mismatch", bytes, Discovery.probeBytes())
                    matched++
                }
                "discovery_reply" -> {
                    val (tcpPort, name) = Discovery.parseReply(bytes, bytes.size)!!
                    assertEquals(fields.getInt("tcp_port"), tcpPort)
                    assertEquals(fields.getString("device_name"), name)
                    val reencoded = Discovery.replyBytes(tcpPort, name)
                    assertArrayEquals("reply encode mismatch", bytes, reencoded)
                    matched++
                }
            }
        }
        assertTrue("匹配的向量过少: $matched", matched >= 6)
    }

    @Test
    fun helloExchangeOverStreams() {
        val hello = Hello(
            deviceName = "TestDevice",
            codec = Protocol.CODEC_PCM_S16LE,
            sampleRate = Protocol.SAMPLE_RATE,
            channels = Protocol.CHANNELS,
            frameMs = Protocol.DEFAULT_FRAME_MS,
            bufferTargetMs = Protocol.DEFAULT_TARGET_MS,
            authTokenSha256 = ByteArray(32),
        )
        val ack = HelloAck(
            status = AckStatus.OK,
            codec = Protocol.CODEC_PCM_S16LE,
            sampleRate = Protocol.SAMPLE_RATE,
            channels = Protocol.CHANNELS,
            deviceName = "DESKTOP-PC",
            jitterTargetMs = Protocol.DEFAULT_TARGET_MS,
        )
        val out = ByteArrayOutputStream()
        sendHello(out, hello)
        sendHelloAck(out, ack)
        val input = ByteArrayInputStream(out.toByteArray())
        assertEquals(hello.toString(), readHello(input).toString())
        assertEquals(ack.toString(), readHelloAck(input).toString())
    }

    @Test
    fun rejectedAckThrows() {
        val out = ByteArrayOutputStream()
        sendHelloAck(
            out,
            HelloAck(AckStatus.BUSY, Protocol.CODEC_PCM_S16LE, 48_000, 2, "PC", 80),
        )
        val input = ByteArrayInputStream(out.toByteArray())
        try {
            readHelloAck(input)
            error("should have thrown")
        } catch (e: ProtocolException) {
            assertTrue(e.message!!.contains("BUSY"))
        }
    }

    @Test
    fun byeAndPingHelpers() {
        val bye = byePayload(0, "bye")
        assertEquals(0 to "bye", parseByePayload(bye))
        assertEquals((0L to "bye"), parseByePayload(bye).let { it.first.toLong() to it.second })
        val ping = pingPayload(0xDEADBEEFL)
        assertEquals(0xDEADBEEFL, parsePingPayload(ping)!!)
    }

    @Test
    fun truncatedBuffersRejected() {
        val bytes = Hello(
            "x", Protocol.CODEC_PCM_S16LE, 48_000, 2, 20, 80, ByteArray(32),
        ).toBytes()
        for (cut in intArrayOf(0, 1, 5, 40)) {
            try {
                Hello.fromBytes(bytes.copyOf(cut))
                if (cut < bytes.size) error("cut=$cut should have thrown")
            } catch (_: ProtocolException) {
                // 预期
            }
        }
    }
}
