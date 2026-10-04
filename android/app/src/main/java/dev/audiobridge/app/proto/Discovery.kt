package dev.audiobridge.app.proto

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.SocketTimeoutException
import java.util.concurrent.atomic.AtomicBoolean

/**
 * UDP 广播设备发现（audiobridge-core discovery.rs 的 Kotlin 镜像）。
 *
 * - 探测包：`ABRQ` + 版本 u8，发往全局广播、各子网定向广播与 127.0.0.1；
 * - 应答包：`ABRP` + 版本 u8 + tcp_port u16 + name_len u16 + name，
 *   接收端向探测方单播。
 * 目的端口 = 服务端口（默认 48000），UDP 与 TCP 同端口不同协议栈。
 */
object Discovery {
    val MAGIC_PROBE = byteArrayOf(0x41, 0x42, 0x52, 0x51) // "ABRQ"
    val MAGIC_REPLY = byteArrayOf(0x41, 0x42, 0x52, 0x50) // "ABRP"

    const val MAX_NAME_LEN = 255

    data class DiscoveredDevice(
        val addr: String,
        val tcpPort: Int,
        val name: String,
    )

    fun probeBytes(): ByteArray = byteArrayOf(
        MAGIC_PROBE[0], MAGIC_PROBE[1], MAGIC_PROBE[2], MAGIC_PROBE[3],
        Protocol.PROTOCOL_VERSION.toByte(),
    )

    /** 构造应答包（超长名按 UTF-8 字符边界截断，与 Rust 一致）。 */
    fun replyBytes(tcpPort: Int, name: String): ByteArray {
        val raw = name.toByteArray(Charsets.UTF_8)
        var len = minOf(raw.size, MAX_NAME_LEN)
        // 仅在发生截断时检查切点是否落在多字节字符中间
        while (len > 0 && len < raw.size && (raw[len].toInt() and 0xC0) == 0x80) len--
        val out = ByteArray(4 + 1 + 2 + 2 + len)
        System.arraycopy(MAGIC_REPLY, 0, out, 0, 4)
        out[4] = Protocol.PROTOCOL_VERSION.toByte()
        out[5] = (tcpPort and 0xFF).toByte()
        out[6] = ((tcpPort shr 8) and 0xFF).toByte()
        out[7] = (len and 0xFF).toByte()
        out[8] = ((len shr 8) and 0xFF).toByte()
        System.arraycopy(raw, 0, out, 9, len)
        return out
    }

    /** 校验探测包是否合法（`ABRQ` + 版本号）。 */
    fun parseProbe(buf: ByteArray, len: Int): Boolean {
        if (len < 5 || len > buf.size) return false
        for (i in 0..3) if (buf[i] != MAGIC_PROBE[i]) return false
        return (buf[4].toInt() and 0xFF) == Protocol.PROTOCOL_VERSION
    }

    /** 解析应答包，返回 `(tcpPort, name)`；非法包返回 null。 */
    fun parseReply(buf: ByteArray, len: Int): Pair<Int, String>? {
        if (len < 9 || len > buf.size) return null
        for (i in 0..3) if (buf[i] != MAGIC_REPLY[i]) return null
        if ((buf[4].toInt() and 0xFF) != Protocol.PROTOCOL_VERSION) return null
        val tcpPort = (buf[5].toInt() and 0xFF) or ((buf[6].toInt() and 0xFF) shl 8)
        val nameLen = (buf[7].toInt() and 0xFF) or ((buf[8].toInt() and 0xFF) shl 8)
        if (len < 9 + nameLen) return null
        val name = String(buf, 9, nameLen, Charsets.UTF_8)
        return tcpPort to name
    }

    private fun probeTargets(): List<InetAddress> {
        val out = mutableListOf<InetAddress>()
        out.add(InetAddress.getByName("255.255.255.255"))
        out.add(InetAddress.getByName("127.0.0.1"))
        try {
            val it = NetworkInterface.getNetworkInterfaces()
            while (it.hasMoreElements()) {
                val nif = it.nextElement()
                for (ia in nif.interfaceAddresses) {
                    val bc = ia.broadcast ?: continue
                    if (!out.contains(bc)) out.add(bc)
                }
            }
        } catch (_: Exception) {
        }
        return out
    }

    /**
     * 阻塞式扫描（必须在非主线程调用）。发探测包并收集 `timeoutMs` 内的应答，
     * 按地址去重。任何网络异常都返回已收集结果（可为空列表）。
     */
    fun scanBlocking(discoveryPort: Int, timeoutMs: Long): List<DiscoveredDevice> {
        val found = LinkedHashMap<String, DiscoveredDevice>()
        val sock = try {
            DatagramSocket().apply {
                broadcast = true
                soTimeout = 150
            }
        } catch (_: Exception) {
            return emptyList()
        }
        try {
            val probe = probeBytes()
            for (t in probeTargets()) {
                runCatching {
                    sock.send(DatagramPacket(probe, probe.size, t, discoveryPort))
                }
            }
            val deadline = System.currentTimeMillis() + timeoutMs
            val buf = ByteArray(1024)
            while (System.currentTimeMillis() < deadline) {
                val pkt = DatagramPacket(buf, buf.size)
                try {
                    sock.receive(pkt)
                } catch (_: SocketTimeoutException) {
                    continue
                } catch (_: Exception) {
                    break
                }
                val (tcpPort, name) = parseReply(buf, pkt.length) ?: continue
                val key = pkt.address.hostAddress ?: continue
                if (!found.containsKey(key)) {
                    found[key] = DiscoveredDevice(key, tcpPort, name)
                }
            }
        } finally {
            sock.close()
        }
        return found.values.toList()
    }

    /** 接收端常驻应答线程（daemon），`stop` 置位后退出。 */
    fun spawnResponder(
        discoveryPort: Int,
        tcpPort: Int,
        deviceName: String,
        stop: AtomicBoolean,
    ): Thread {
        val t = Thread({
            val sock = try {
                DatagramSocket(null).apply {
                    reuseAddress = true
                    bind(java.net.InetSocketAddress(discoveryPort))
                }
            } catch (e: Exception) {
                android.util.Log.w("ab-discover", "responder bind $discoveryPort failed: ${e.message}")
                return@Thread
            }
            val reply = replyBytes(tcpPort, deviceName)
            android.util.Log.i("ab-discover", "responder listening udp/$discoveryPort")
            val buf = ByteArray(128)
            sock.soTimeout = 500
            while (!stop.get()) {
                val pkt = DatagramPacket(buf, buf.size)
                try {
                    sock.receive(pkt)
                } catch (_: SocketTimeoutException) {
                    continue
                } catch (_: Exception) {
                    break
                }
                val ok = pkt.length >= 5 &&
                    (buf[4].toInt() and 0xFF) == Protocol.PROTOCOL_VERSION &&
                    buf[0] == MAGIC_PROBE[0] && buf[1] == MAGIC_PROBE[1] &&
                    buf[2] == MAGIC_PROBE[2] && buf[3] == MAGIC_PROBE[3]
                if (ok) {
                    runCatching {
                        sock.send(DatagramPacket(reply, reply.size, pkt.address, pkt.port))
                    }
                }
            }
            sock.close()
        }, "ab-discover")
        t.isDaemon = true
        t.start()
        return t
    }
}
