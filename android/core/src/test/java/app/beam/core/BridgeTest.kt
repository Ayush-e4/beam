package app.beam.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** Calls into the Rust engine through the generated bindings, built for this computer. */
class BridgeTest {
    @Test
    fun `version info comes from the engine`() {
        val info = versionInfo()
        assertTrue(info.engine.isNotBlank())
        assertEquals(1.toUShort(), info.protocolMajor)
    }

    @Test
    fun `a newer minor protocol negotiates down to ours`() {
        val ours = versionInfo()
        val agreed = negotiateProtocol(ours.protocolMajor, (ours.protocolMinor + 1u).toUShort())
        assertEquals(ours.protocolMinor, agreed)
    }

    @Test
    fun `engine errors arrive as typed exceptions`() {
        val error =
            assertThrows(BeamException.Protocol::class.java) {
                negotiateProtocol(99u, 0u)
            }
        assertTrue(error.detail, error.detail.contains("incompatible protocol version"))
    }
}
