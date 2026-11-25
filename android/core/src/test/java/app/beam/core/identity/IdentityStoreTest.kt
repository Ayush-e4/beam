package app.beam.core.identity

import java.io.File
import java.security.GeneralSecurityException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Stands in for Android Keystore on the JVM: AES-GCM with an in-memory key. */
private class SoftwareKeyWrapper : KeyWrapper {
    var key: SecretKey = newKey()
    var wrapCount = 0

    override fun wrap(plain: ByteArray): ByteArray {
        wrapCount++
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        return cipher.iv + cipher.doFinal(plain)
    }

    override fun unwrap(wrapped: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, wrapped, 0, 12))
        return cipher.doFinal(wrapped, 12, wrapped.size - 12)
    }

    companion object {
        fun newKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    }
}

class IdentityStoreTest {
    @get:Rule val folder = TemporaryFolder()

    private val wrapper = SoftwareKeyWrapper()

    private fun file() = File(folder.root, "identity/device-identity.bin")

    private fun store() = IdentityStore(file(), wrapper)

    @Test
    fun `the first launch creates an identity and later launches load it`() {
        val first = store().loadOrCreate()
        assertEquals(IdentityOrigin.CREATED, first.origin)

        val again = store().loadOrCreate()
        assertEquals(IdentityOrigin.LOADED, again.origin)
        assertNull(again.problem)
        assertEquals(first.identity.deviceId(), again.identity.deviceId())
        assertArrayEquals(first.identity.publicKey(), again.identity.publicKey())
        assertEquals(1, wrapper.wrapCount)
    }

    @Test
    fun `the private key is never written in the clear`() {
        val identity = store().loadOrCreate().identity
        val stored = file().readBytes()
        val privateKey = identity.privateKey()
        val text = stored.toList()
        assertFalse(text.windowed(privateKey.size).any { it == privateKey.toList() })
        assertFalse(File(file().path + ".tmp").exists())
    }

    @Test
    fun `a lost wrapping key replaces the identity and keeps the old file`() {
        val old = store().loadOrCreate().identity
        wrapper.key = SoftwareKeyWrapper.newKey()

        val result = store().loadOrCreate()
        assertEquals(IdentityOrigin.REPLACED, result.origin)
        assertTrue(result.problem is GeneralSecurityException)
        assertNotEquals(old.deviceId(), result.identity.deviceId())
        assertTrue(File(file().path + ".unreadable").exists())

        // The replacement is stored properly and loads next time.
        assertEquals(IdentityOrigin.LOADED, store().loadOrCreate().origin)
    }

    @Test
    fun `a damaged file replaces the identity`() {
        store().loadOrCreate()
        val bytes = file().readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1] + 1).toByte()
        file().writeBytes(bytes)

        val result = store().loadOrCreate()
        assertEquals(IdentityOrigin.REPLACED, result.origin)
        assertNotNull(result.problem)
    }

    @Test
    fun `an unknown file format replaces the identity`() {
        file().parentFile?.mkdirs()
        file().writeBytes(byteArrayOf(9, 1, 2, 3))

        val result = store().loadOrCreate()
        assertEquals(IdentityOrigin.REPLACED, result.origin)
        assertEquals("unknown identity file format", result.problem?.message)
    }
}
