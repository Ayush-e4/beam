package app.beam.core.identity

import app.beam.core.BeamException
import app.beam.core.DeviceIdentity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Rust identity object, through the generated bindings. */
class DeviceIdentityTest {
    @Test
    fun `keys and device IDs have the documented shape`() {
        val identity = DeviceIdentity.generate()
        assertEquals(32, identity.publicKey().size)
        assertEquals(32, identity.privateKey().size)
        assertTrue(identity.deviceId().matches(Regex("[0-9a-f]{32}")))
    }

    @Test
    fun `a private key restores the same device`() {
        val original = DeviceIdentity.generate()
        val restored = DeviceIdentity.fromPrivateKey(original.privateKey())
        assertEquals(original.deviceId(), restored.deviceId())
        assertNotEquals(original.deviceId(), DeviceIdentity.generate().deviceId())
    }

    @Test
    fun `a malformed private key is a crypto error`() {
        assertThrows(BeamException.Crypto::class.java) {
            DeviceIdentity.fromPrivateKey(ByteArray(5))
        }
    }
}
