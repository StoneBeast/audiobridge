package dev.audiobridge.app.proto

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicBoolean

/** UDP 设备发现测试：应答器 + 扫描在本进程内闭环（与 Rust discovery 测试对应）。 */
class DiscoveryTest {

    private fun freeUdpPort(): Int =
        java.net.DatagramSocket(0).let { s ->
            val p = s.localPort
            s.close()
            p
        }

    @Test
    fun probeAndReplyRoundtrip() {
        val probe = Discovery.probeBytes()
        assertTrue(Discovery.parseProbe(probe, probe.size))
        // 非法：错魔数 / 错版本 / 截断
        val bad = probe.copyOf().also { it[0] = 0x00 }
        assertFalse(Discovery.parseProbe(bad, bad.size))
        val badVer = probe.copyOf().also { it[4] = 0x09 }
        assertFalse(Discovery.parseProbe(badVer, badVer.size))
        assertFalse(Discovery.parseProbe(probe, 3))

        val reply = Discovery.replyBytes(48_000, "Pixel 8")
        val parsed = Discovery.parseReply(reply, reply.size)
        assertNotNull(parsed)
        assertEquals(48_000, parsed!!.first)
        assertEquals("Pixel 8", parsed.second)
        // 错魔数 / 截断
        assertNull(Discovery.parseReply(reply.copyOf().also { it[0] = 0x00 }, reply.size))
        assertNull(Discovery.parseReply(reply, 7))
    }

    @Test
    fun longNameTruncatesAtUtf8Boundary() {
        val long = "很长的设备名 ".repeat(40)
        val reply = Discovery.replyBytes(1, long)
        val (_, name) = Discovery.parseReply(reply, reply.size)!!
        assertTrue(name.toByteArray(Charsets.UTF_8).size <= Discovery.MAX_NAME_LEN)
        assertTrue(long.startsWith(name))
    }

    @Test
    fun scanFindsLoopbackResponder() {
        val port = freeUdpPort()
        val stop = AtomicBoolean(false)
        Discovery.spawnResponder(port, port, "TEST-RECEIVER", stop)
        Thread.sleep(300)
        try {
            val found = Discovery.scanBlocking(port, 1500)
            val hit = found.find {
                (it.addr == "127.0.0.1" || it.addr == "0:0:0:0:0:0:0:1") &&
                    it.name == "TEST-RECEIVER" && it.tcpPort == port
            }
            assertNotNull("loopback responder not found: $found", hit)
        } finally {
            stop.set(true)
        }
    }
}
