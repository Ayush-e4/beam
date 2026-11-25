package app.beam.core.identity

import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Wraps keys with an AES-256-GCM key that lives in Android Keystore and never leaves it: in
 * StrongBox when the phone has one, otherwise in the TEE. The key needs no user authentication,
 * because transfers run in the background while the phone is locked.
 *
 * Layout of a wrapped key: 12-byte IV, then ciphertext with a 16-byte tag.
 */
class KeystoreKeyWrapper(private val alias: String = DEFAULT_ALIAS) : KeyWrapper {
    override fun wrap(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, existingKey() ?: createKey())
        cipher.updateAAD(AAD)
        val iv = cipher.iv
        check(iv.size == IV_LEN) { "Keystore chose a ${iv.size}-byte IV" }
        return iv + cipher.doFinal(plain)
    }

    override fun unwrap(wrapped: ByteArray): ByteArray {
        if (wrapped.size < IV_LEN + TAG_BITS / 8) {
            throw GeneralSecurityException("wrapped key is too short (${wrapped.size} bytes)")
        }
        val key = existingKey() ?: throw GeneralSecurityException("the wrapping key is gone")
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, wrapped, 0, IV_LEN))
        cipher.updateAAD(AAD)
        return cipher.doFinal(wrapped, IV_LEN, wrapped.size - IV_LEN)
    }

    /** Removes the wrapping key, making every key wrapped with it unreadable. */
    fun deleteKey() {
        keyStore().deleteEntry(alias)
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

    private fun existingKey(): SecretKey? = keyStore().getKey(alias, null) as SecretKey?

    private fun createKey(): SecretKey {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                return generate(strongBox = true)
            } catch (_: StrongBoxUnavailableException) {
                // No secure element on this phone; the TEE-backed key below is the fallback.
            }
        }
        return generate(strongBox = false)
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec =
            KeyGenParameterSpec
                .Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(KEY_BITS)
                .apply {
                    if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                        setIsStrongBoxBacked(true)
                    }
                }.build()
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(spec)
        return generator.generateKey()
    }

    companion object {
        /** Alias of the key that wraps the device identity. */
        const val DEFAULT_ALIAS = "beam.device-identity.wrap.v1"
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val KEY_BITS = 256
        private const val TAG_BITS = 128
        private const val IV_LEN = 12

        /** Binds wrapped blobs to this use, so one can't be passed off as another kind of key. */
        private val AAD = "beam device identity v1".encodeToByteArray()
    }
}
