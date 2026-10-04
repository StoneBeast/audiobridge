package dev.audiobridge.app.util

import java.security.MessageDigest

/** 访问令牌 -> SHA-256 摘要（与 PC 端算法一致：摘要 = SHA-256(UTF-8(令牌))）。 */
fun sha256Token(token: String): ByteArray {
    if (token.isEmpty()) return ByteArray(32)
    return MessageDigest.getInstance("SHA-256").digest(token.toByteArray(Charsets.UTF_8))
}
