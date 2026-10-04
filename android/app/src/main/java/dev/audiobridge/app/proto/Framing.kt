package dev.audiobridge.app.proto

import java.io.EOFException
import java.io.InputStream
import java.io.OutputStream

/**
 * 流层面的读写：握手（HELLO/HELLO_ACK）与握手后的流式消息帧。
 *
 * 帧格式见 docs/protocol.md：
 * - 握手帧：`[magic 4B][version 1B][type 1B][body_len 2B][body]`
 * - 流式帧：`[type 1B][payload_len 2B][payload]`
 */

/** 握手完成后的流式消息。 */
class Message(val type: Int, val payload: ByteArray) {
    override fun toString(): String =
        "Message(${Protocol.msgTypeName(type)}, ${payload.size}B)"
}

/** 发送端 -> 接收端：发送 HELLO。 */
fun sendHello(out: OutputStream, hello: Hello) =
    writeHandshake(out, Protocol.MSG_HELLO, hello.toBytes())

/** 接收端 -> 发送端：发送 HELLO_ACK。 */
fun sendHelloAck(out: OutputStream, ack: HelloAck) =
    writeHandshake(out, Protocol.MSG_HELLO_ACK, ack.toBytes())

/** 接收端：读取并校验 HELLO。 */
fun readHello(input: InputStream): Hello =
    Hello.fromBytes(readHandshakeBody(input, Protocol.MSG_HELLO))

/** 发送端：读取 HELLO_ACK；status != OK 时抛出 [ProtocolException]。 */
fun readHelloAck(input: InputStream): HelloAck {
    val ack = HelloAck.fromBytes(readHandshakeBody(input, Protocol.MSG_HELLO_ACK))
    if (ack.status != AckStatus.OK) {
        throw ProtocolException("handshake rejected: ${AckStatus.name(ack.status)}")
    }
    return ack
}

/** 发送端发起连接的完整握手：发送 HELLO 并等待 HELLO_ACK。 */
fun handshake(out: OutputStream, input: InputStream, hello: Hello): HelloAck {
    sendHello(out, hello)
    return readHelloAck(input)
}

private fun writeHandshake(out: OutputStream, type: Int, body: ByteArray) {
    if (body.size > Protocol.MAX_PAYLOAD) throw ProtocolException("handshake body too large: ${body.size}")
    out.write(Protocol.MAGIC)
    out.write(byteArrayOf(Protocol.PROTOCOL_VERSION.toByte(), type.toByte()))
    writeU16(out, body.size)
    out.write(body)
    out.flush()
}

private fun readHandshakeBody(input: InputStream, expectedType: Int): ByteArray {
    val head = readFully(input, 8)
    for (i in 0..3) {
        if (head[i] != Protocol.MAGIC[i]) throw ProtocolException("bad magic at byte $i: 0x%02x".format(head[i]))
    }
    val version = head[4].toInt() and 0xFF
    if (version != Protocol.PROTOCOL_VERSION) {
        throw ProtocolException("unsupported protocol version: $version")
    }
    val type = head[5].toInt() and 0xFF
    if (type != expectedType) {
        throw ProtocolException("unexpected message type ${Protocol.msgTypeName(type)}, want ${Protocol.msgTypeName(expectedType)}")
    }
    val len = ((head[6].toInt() and 0xFF) or ((head[7].toInt() and 0xFF) shl 8))
    return readFully(input, len)
}

/** 写出一条流式消息帧。 */
fun sendMessage(out: OutputStream, msgType: Int, payload: ByteArray) {
    if (payload.size > Protocol.MAX_PAYLOAD) throw ProtocolException("payload too large: ${payload.size}")
    out.write(msgType)
    writeU16(out, payload.size)
    out.write(payload)
    out.flush()
}

/** 读入一条流式消息帧。流结束抛 EOFException。 */
fun readMessage(input: InputStream): Message {
    val hdr = readFully(input, 3)
    val type = hdr[0].toInt() and 0xFF
    val len = ((hdr[1].toInt() and 0xFF) or ((hdr[2].toInt() and 0xFF) shl 8))
    return Message(type, readFully(input, len))
}

private fun writeU16(out: OutputStream, v: Int) {
    out.write(v and 0xFF)
    out.write((v shr 8) and 0xFF)
}

/** 精确读取 n 字节，不足（对端关闭）时抛 EOFException。 */
fun readFully(input: InputStream, n: Int): ByteArray {
    val out = ByteArray(n)
    var off = 0
    while (off < n) {
        val r = input.read(out, off, n - off)
        if (r < 0) throw EOFException("stream ended after $off of $n bytes")
        off += r
    }
    return out
}

/** BYE 负载：[reason u8][可选 UTF-8 说明]。 */
fun byePayload(reason: Int, note: String): ByteArray =
    ByteWriter().u8(reason).bytes(note.toByteArray(Charsets.UTF_8)).toByteArray()

fun parseByePayload(payload: ByteArray): Pair<Int, String> {
    if (payload.isEmpty()) return 0 to ""
    return (payload[0].toInt() and 0xFF) to String(payload, 1, payload.size - 1, Charsets.UTF_8)
}

/** PING/PONG 负载：4 字节 nonce。 */
fun pingPayload(nonce: Long): ByteArray = ByteWriter().u32(nonce).toByteArray()

fun parsePingPayload(payload: ByteArray): Long? =
    if (payload.size < 4) null else ByteReader(payload).u32()
