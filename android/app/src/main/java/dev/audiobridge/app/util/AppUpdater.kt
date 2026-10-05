package dev.audiobridge.app.util

import android.content.Context
import android.content.Intent
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * 应用内更新：检查 GitHub Release 的 update.json → 下载 APK → 调起系统安装。
 *
 * 首次安装更新时系统会要求授予「安装未知应用」权限，跟随系统提示操作即可。
 * 必须在非主线程调用 check/download（网络操作）。
 */
object AppUpdater {
    const val UPDATE_URL: String =
        "https://github.com/StoneBeast/audiobridge/releases/latest/download/update.json"
    const val RELEASES_PAGE: String =
        "https://github.com/StoneBeast/audiobridge/releases/latest"

    data class Info(
        val version: String,
        val notes: String,
        val androidUrl: String,
    )

    fun currentVersion(context: Context): String = try {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0.0.0"
    } catch (_: Exception) {
        "0.0.0"
    }

    /** x.y.z 数值比较。 */
    fun isNewer(candidate: String, current: String): Boolean {
        fun parts(v: String) = v.trim().removePrefix("v").split(".")
            .map { it.trim().toIntOrNull() ?: 0 }
        val a = parts(candidate)
        val b = parts(current)
        for (i in 0..2) {
            val x = a.getOrElse(i) { 0 }
            val y = b.getOrElse(i) { 0 }
            if (x != y) return x > y
        }
        return false
    }

    /** 检查更新；有新版本返回 Info，否则 null。 */
    fun check(context: Context): Info? {
        val conn = URL(UPDATE_URL).openConnection() as HttpURLConnection
        conn.connectTimeout = 10_000
        conn.readTimeout = 10_000
        conn.instanceFollowRedirects = true
        try {
            if (conn.responseCode != 200) return null
            val body = conn.inputStream.bufferedReader().readText()
            val o = JSONObject(body)
            val info = Info(
                version = o.optString("version"),
                notes = o.optString("notes"),
                androidUrl = o.optString("android_url"),
            )
            if (!isNewer(info.version, currentVersion(context))) return null
            if (info.androidUrl.isBlank()) return null
            return info
        } finally {
            conn.disconnect()
        }
    }

    /** 流式下载 APK 到应用外部私有目录 update/（无需存储权限），返回文件。 */
    fun download(context: Context, url: String, onProgress: (Long, Long) -> Unit): File {
        val dir = File(context.getExternalFilesDir(null), "update").apply { mkdirs() }
        val dest = File(dir, "AudioBridge-new.apk")
        val tmp = File(dir, "AudioBridge-new.apk.part")
        val conn = URL(url).openConnection() as HttpURLConnection
        conn.connectTimeout = 15_000
        conn.readTimeout = 30_000
        conn.instanceFollowRedirects = true
        try {
            if (conn.responseCode != 200) {
                throw java.io.IOException("下载失败: HTTP ${conn.responseCode}")
            }
            val total = conn.contentLengthLong
            conn.inputStream.use { input ->
                java.io.FileOutputStream(tmp).use { out ->
                    val buf = ByteArray(64 * 1024)
                    var got = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        got += n
                        onProgress(got, total)
                    }
                }
            }
            if (tmp.length() == 0L) throw java.io.IOException("下载内容为空")
            if (dest.exists()) dest.delete()
            if (!tmp.renameTo(dest)) {
                tmp.copyTo(dest, overwrite = true)
                tmp.delete()
            }
            return dest
        } finally {
            conn.disconnect()
        }
    }

    /** 调起系统包安装器。 */
    fun installApk(context: Context, apk: File) {
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", apk)
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }
}
