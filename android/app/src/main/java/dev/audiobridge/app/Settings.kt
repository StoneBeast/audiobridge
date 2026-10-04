package dev.audiobridge.app

import android.content.Context
import android.content.SharedPreferences

/** 轻量设置持久化（SharedPreferences）。 */
object Settings {
    private const val NAME = "audiobridge"
    private const val KEY_HOST = "host"
    private const val KEY_PORT = "port"
    private const val KEY_LISTEN_PORT = "listen_port"
    private const val KEY_JITTER_MS = "jitter_ms"
    private const val KEY_TOKEN = "token"

    private var prefs: SharedPreferences? = null

    fun init(context: Context) {
        if (prefs == null) {
            prefs = context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)
        }
    }

    private fun p(): SharedPreferences =
        prefs ?: error("Settings.init(context) 未调用")

    /** 发送模式目标主机（PC 的 IP，或 USB 场景的 127.0.0.1）。 */
    var host: String
        get() = p().getString(KEY_HOST, "192.168.1.100") ?: "192.168.1.100"
        set(v) = p().edit().putString(KEY_HOST, v).apply()

    /** 发送模式目标端口。 */
    var port: Int
        get() = p().getInt(KEY_PORT, 48000)
        set(v) = p().edit().putInt(KEY_PORT, v).apply()

    /** 接收模式监听端口。 */
    var listenPort: Int
        get() = p().getInt(KEY_LISTEN_PORT, 48000)
        set(v) = p().edit().putInt(KEY_LISTEN_PORT, v).apply()

    /** 接收模式抖动缓冲目标水位（毫秒）。 */
    var jitterMs: Int
        get() = p().getInt(KEY_JITTER_MS, ProtocolDefaults.TARGET_MS)
        set(v) = p().edit().putInt(KEY_JITTER_MS, v).apply()

    /** 可选访问令牌（与 PC 端设置一致才能连接）。 */
    var token: String
        get() = p().getString(KEY_TOKEN, "") ?: ""
        set(v) = p().edit().putString(KEY_TOKEN, v).apply()

    object ProtocolDefaults {
        const val TARGET_MS = 80
    }
}
