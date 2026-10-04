package dev.audiobridge.app.proto

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 抖动缓冲行为测试（与 audiobridge-core jitter.rs 的测试一一对应）。 */
class JitterBufferTest {

    // 48kHz 立体声 S16 = 192 B/ms；20ms 帧 = 3840B；目标 80ms；容量 200ms
    private fun buffer() = JitterBuffer(48_000, 2, 20, 80, 200)

    @Test
    fun primesBeforePlaying() {
        val jb = buffer()
        val out = ByteArray(3840)
        assertEquals(0, jb.pop(out)) // 未注水
        repeat(3) { jb.push(ByteArray(3840) { 1 }) }
        assertFalse(jb.isStarted()) // 60ms < 80ms
        assertEquals(0, jb.pop(out))
        jb.push(ByteArray(3840) { 1 }) // 达到 80ms
        assertEquals(3840, jb.pop(out))
        assertTrue(jb.isStarted())
        assertEquals(60, jb.bufferedMs())
    }

    @Test
    fun partialPopAcrossChunks() {
        val jb = buffer()
        repeat(5) { jb.push(ByteArray(3840) { 7 }) }
        val half = ByteArray(1920)
        assertEquals(1920, jb.pop(half))
        assertTrue(half.all { it == 7.toByte() })
        val full = ByteArray(3840)
        assertEquals(3840, jb.pop(full))
        assertTrue(full.all { it == 7.toByte() })
    }

    @Test
    fun underrunCountsOncePerDrain() {
        val jb = buffer()
        repeat(5) { jb.push(ByteArray(3840) { 1 }) }
        val out = ByteArray(3840)
        repeat(5) { assertEquals(3840, jb.pop(out)) }
        assertEquals(0, jb.pop(out))
        assertEquals(1, jb.stats().underruns)
        assertEquals(0, jb.pop(out))
        assertEquals(1, jb.stats().underruns) // 持续欠载只记一次
        jb.push(ByteArray(3840) { 1 })
        assertEquals(3840, jb.pop(out)) // 数据恢复立即续播
        assertEquals(1, jb.stats().underruns)
    }

    @Test
    fun overflowDropsOldest() {
        val jb = buffer()
        for (i in 0..10) {
            jb.push(ByteArray(3840) { i.toByte() }) // 11 帧 > 容量 10 帧
        }
        assertEquals(3840L, jb.stats().droppedBytes)
        val out = ByteArray(3840)
        jb.pop(out)
        assertTrue(out.all { it == 1.toByte() }) // 最旧的 0 已丢弃
    }

    @Test
    fun pushWithOffsetAndLength() {
        val jb = buffer()
        val data = ByteArray(7680) { if (it < 3840) 1 else 2 }
        jb.push(data, 3840, 3840)
        val out = ByteArray(3840)
        assertEquals(0, jb.pop(out)) // 未达目标
        jb.push(data, 3840, 3840)
        jb.push(data, 3840, 3840)
        jb.push(data, 3840, 3840)
        assertEquals(3840, jb.pop(out))
    }

    @Test
    fun resetClearsButKeepsStats() {
        val jb = buffer()
        repeat(5) { jb.push(ByteArray(3840) { 1 }) }
        val out = ByteArray(3840)
        jb.pop(out)
        val poppedBefore = jb.stats().poppedBytes
        jb.reset()
        assertEquals(0, jb.bufferedMs())
        assertFalse(jb.isStarted())
        assertEquals(poppedBefore, jb.stats().poppedBytes)
    }
}
