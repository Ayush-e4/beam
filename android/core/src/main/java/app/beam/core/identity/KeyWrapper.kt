package app.beam.core.identity

import java.security.GeneralSecurityException

/** Encrypts the device's private key for storage, so the file alone is useless. */
interface KeyWrapper {
    /** Returns [plain] encrypted and authenticated; the caller wipes [plain] afterwards. */
    @Throws(GeneralSecurityException::class)
    fun wrap(plain: ByteArray): ByteArray

    /** Reverses [wrap]; throws if [wrapped] was altered or the wrapping key is gone. */
    @Throws(GeneralSecurityException::class)
    fun unwrap(wrapped: ByteArray): ByteArray
}
