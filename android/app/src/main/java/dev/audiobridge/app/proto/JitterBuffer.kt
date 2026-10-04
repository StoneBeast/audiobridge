package dev.audiobridge.app.proto

/**
 * 接收端抖动缓冲（audiobridge-core jitter.rs 的 Kotlin 镜像）。
 *
 * - 初始注水：缓冲达到 targetMs 之前 [pop] 只返回 0（调用方补静音）；
 * - 欠载：已启动但耗尽时补静音并计 underruns，数据恢复立即续播；
 * - 过载：超出 maxMs 容量时从最旧端丢弃。
 *
 * 线程模型：网络读线程 push，音频播放线程 pop，全部方法 @Synchronized，
 * 临界区为内存拷贝，极短。
 */
class JitterBuffer(
    sampleRate: Int,
    channels: Int,
    frameMs: Int,
    targetMs: Int,
    maxMs: Int,
) {
    private val queue = ArrayDeque<ByteArray>()
    private val bytesPerMs: Int =
        Protocol.s16BytesPerMs(maxOf(1, sampleRate), maxOf(1, channels)).coerceAtLeast(1)
    val chunkBytes: Int = bytesPerMs * maxOf(1, frameMs)
    private val targetBytes: Int = bytesPerMs * maxOf(0, targetMs)
    private val capacityBytes: Int =
        maxOf(bytesPerMs * maxOf(1, maxMs), chunkBytes)
    private var started = false

    private var pushedBytesTotal = 0L
    private var poppedBytesTotal = 0L
    private var droppedBytesTotal = 0L
    private var underrunsTotal = 0L

    data class Stats(
        val pushedBytes: Long,
        val poppedBytes: Long,
        val droppedBytes: Long,
        val underruns: Long,
    )

    /** 写入音频字节（网络线程调用）。 */
    @Synchronized
    fun push(data: ByteArray, off: Int = 0, len: Int = data.size - off) {
        require(off >= 0 && len >= 0 && off + len <= data.size) { "bad slice off=$off len=$len size=${data.size}" }
        if (len == 0) return
        pushedBytesTotal += len
        var overflow = maxOf(0, bufferedBytesInternal() + len - capacityBytes)
        while (overflow > 0) {
            val front = queue.firstOrNull() ?: break
            if (front.size <= overflow) {
                queue.removeFirst()
                droppedBytesTotal += front.size
                overflow -= front.size
            } else {
                queue[0] = front.copyOfRange(overflow, front.size)
                droppedBytesTotal += overflow
                overflow = 0
            }
        }
        queue.addLast(data.copyOfRange(off, off + len))
    }

    /**
     * 读出至多 out.size 字节填充 out，不足部分补静音；
     * 返回实际取自缓冲的字节数（0 = 完全欠载或尚未注水完成）。
     */
    @Synchronized
    fun pop(out: ByteArray): Int {
        java.util.Arrays.fill(out, 0)
        if (out.isEmpty()) return 0
        if (!started) {
            if (bufferedBytesInternal() >= targetBytes) started = true else return 0
        }
        val wasEmpty = queue.isEmpty()
        var filled = 0
        var cursor = 0
        while (cursor < out.size) {
            val front = queue.firstOrNull() ?: break
            val take = minOf(front.size, out.size - cursor)
            System.arraycopy(front, 0, out, cursor, take)
            cursor += take
            if (take == front.size) {
                queue.removeFirst()
            } else {
                queue[0] = front.copyOfRange(take, front.size)
            }
            filled += take
        }
        if (wasEmpty && filled == 0) underrunsTotal += 1
        poppedBytesTotal += filled
        return filled
    }

    @Synchronized
    fun bufferedMs(): Long = bufferedBytesInternal() / bytesPerMs.toLong()

    @Synchronized
    fun isStarted(): Boolean = started

    @Synchronized
    fun stats(): Stats = Stats(pushedBytesTotal, poppedBytesTotal, droppedBytesTotal, underrunsTotal)

    /** 清空缓冲（新会话开始时），统计保留。 */
    @Synchronized
    fun reset() {
        queue.clear()
        started = false
    }

    private fun bufferedBytesInternal(): Int = queue.sumOf { it.size }
}
