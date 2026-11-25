package app.beam.core

import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pairing two engines through the generated Kotlin API. */
class PairingTest {
    @Test
    fun `two engines pair and agree on the verification code`() = runBlocking {
        val a = DeviceIdentity.generate()
        val b = DeviceIdentity.generate()
        Engine(a).use { engineA ->
            Engine(b).use { engineB ->
                val host = engineA.hostPairing(SessionCipher.AES_GCM)
                val code = host.code(listOf("127.0.0.1"))
                assertTrue(code, code.startsWith("BEAM1:"))
                val shown = async { host.waitForDevice("Phone A") }
                val scanned = engineB.pairWith(code, "Phone B")

                assertArrayEquals(a.publicKey(), scanned.publicKey)
                assertEquals("Phone A", scanned.name)
                assertEquals("Phone B", shown.await().name)
                assertEquals(shown.await().verificationCode, scanned.verificationCode)
            }
        }
    }

    @Test
    fun `a code from somewhere else explains itself`() = runBlocking {
        Engine(DeviceIdentity.generate()).use { engine ->
            val error = runCatching {
                engine.pairWith("https://example.com", "Phone")
            }.exceptionOrNull()
            assertTrue("$error", error is BeamException.Protocol)
            assertTrue(
                (error as BeamException.Protocol).detail.contains("not a Beam pairing code")
            )
        }
    }
}
