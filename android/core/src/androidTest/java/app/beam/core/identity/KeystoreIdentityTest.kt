package app.beam.core.identity

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.GeneralSecurityException
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith

/** Runs against the phone's real Android Keystore (StrongBox or TEE). */
@RunWith(AndroidJUnit4::class)
class KeystoreIdentityTest {
    private val wrapper = KeystoreKeyWrapper(alias = "beam.test.identity-wrap")
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val file = File(context.noBackupFilesDir, "test-device-identity.bin")

    @After
    fun cleanUp() {
        wrapper.deleteKey()
        file.delete()
        File(file.path + ".unreadable").delete()
    }

    @Test
    fun wrappingRoundTripsAndHidesTheKey() {
        val plain = ByteArray(32) { it.toByte() }
        val wrapped = wrapper.wrap(plain)
        assertEquals(12 + 32 + 16, wrapped.size)
        assertFalse(wrapped.toList().windowed(32).any { it == plain.toList() })
        assertArrayEquals(plain, wrapper.unwrap(wrapped))
    }

    @Test
    fun alteredBlobsAreRejected() {
        val wrapped = wrapper.wrap(ByteArray(32) { 7 })
        wrapped[20] = (wrapped[20] + 1).toByte()
        assertThrows(GeneralSecurityException::class.java) { wrapper.unwrap(wrapped) }
    }

    @Test
    fun theIdentitySurvivesARestartAndNotTheLossOfItsKey() {
        val first = IdentityStore(file, wrapper).loadOrCreate()
        assertEquals(IdentityOrigin.CREATED, first.origin)
        val again = IdentityStore(file, wrapper).loadOrCreate()
        assertEquals(IdentityOrigin.LOADED, again.origin)
        assertEquals(first.identity.deviceId(), again.identity.deviceId())

        wrapper.deleteKey()
        assertEquals(IdentityOrigin.REPLACED, IdentityStore(file, wrapper).loadOrCreate().origin)
    }
}
