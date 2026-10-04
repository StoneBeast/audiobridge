package dev.audiobridge.app

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 服务 <-> UI 的全局状态总线。
 * 服务是状态的唯一写方，Compose UI 通过 collectAsState 观察更新。
 */
object AppBus {

    data class SenderState(
        val running: Boolean = false,
        val connected: Boolean = false,
        val peer: String = "",
        val error: String? = null,
        val sentBytes: Long = 0,
        val sentSeconds: Long = 0,
    )

    data class ReceiverState(
        val running: Boolean = false,
        val clientConnected: Boolean = false,
        val peer: String = "",
        val bufferedMs: Long = 0,
        val underruns: Long = 0,
        val error: String? = null,
    )

    private val _sender = MutableStateFlow(SenderState())
    val sender = _sender.asStateFlow()

    private val _receiver = MutableStateFlow(ReceiverState())
    val receiver = _receiver.asStateFlow()

    fun updateSender(block: (SenderState) -> SenderState) {
        _sender.value = block(_sender.value)
    }

    fun updateReceiver(block: (ReceiverState) -> ReceiverState) {
        _receiver.value = block(_receiver.value)
    }
}
