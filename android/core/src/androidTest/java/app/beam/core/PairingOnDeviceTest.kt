package app.beam.core

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.trust.TrustedDevice
import app.beam.core.trust.TrustedDevices
import java.io.File
import java.net.Inet4Address
import java.net.NetworkInterface
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Pairing on a real phone, the way the app will do it: one engine shows a code carrying the
 * phone's own network addresses, the other pairs from that code, the result is kept as a
 * trusted device, and a file then goes to the trusted key with the cipher chosen at pairing.
 */
@RunWith(AndroidJUnit4::class)
class PairingOnDeviceTest {
    private val root =
        File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "pairing-test")

    @After
    fun cleanUp() {
        root.deleteRecursively()
    }

    @Test
    fun pairedDeviceIsTrustedAndReceivesAFile() = runBlocking {
        val addresses = phoneAddresses()
        Log.i(TAG, "pairing code carries $addresses")
        val laptop = DeviceIdentity.generate()
        val phone = DeviceIdentity.generate()

        Engine(laptop).use { laptopEngine ->
            Engine(phone).use { phoneEngine ->
                val host = laptopEngine.hostPairing(SessionCipher.CHA_CHA_POLY)
                val code = host.code(addresses)
                val shown = async { host.waitForDevice("Laptop") }
                val scanned = phoneEngine.pairWith(code, "Galaxy")

                assertEquals("Galaxy", shown.await().name)
                assertEquals("Laptop", scanned.name)
                assertEquals(shown.await().verificationCode, scanned.verificationCode)
                assertEquals(SessionCipher.CHA_CHA_POLY, scanned.cipher)

                val trust = File(root, "trusted-devices.bin")
                TrustedDevices(trust).add(TrustedDevice.from(scanned, System.currentTimeMillis()))
                val trusted = checkNotNull(
                    TrustedDevices(trust).find(laptop.publicKey())
                ) { "trusted device survives a reload" }

                val laptopTrust = TrustedDevices(File(root, "laptop-trusted-devices.bin"))
                laptopTrust.add(TrustedDevice.from(shown.await(), System.currentTimeMillis()))
                sendFile(phoneEngine, laptopEngine, laptopTrust, trusted, addresses.first())
            }
        }
    }

    /** Sends a file from the phone to the laptop, each using what pairing gave it. */
    private suspend fun sendFile(
        phoneEngine: Engine,
        laptopEngine: Engine,
        laptopTrust: TrustedDevices,
        laptop: TrustedDevice,
        address: String
    ) = coroutineScope {
        val source = File(root, "outbox/note.txt").apply { parentFile?.mkdirs() }
        source.writeText("sent to a paired device")
        val inbox = File(root, "inbox").apply { mkdirs() }
        val listener = laptopEngine.listen(0u, laptop.cipher, laptopTrust)
        val receive = async {
            val offer = listener.nextOffer()
            assertTrue("the phone is in the laptop's trusted list", offer.fromTrustedDevice())
            offer.accept(inbox.path, emptyList(), NoProgress)
        }
        val result = phoneEngine.prepareSend(listOf(source.path))
            .sendTo(address, listener.port(), laptop.publicKey, laptop.cipher, NoProgress)
        receive.await()
        assertEquals(SendResult.Delivered, result)
        assertArrayEquals(source.readBytes(), File(inbox, "note.txt").readBytes())
    }

    /** The phone's own IPv4 addresses, Wi-Fi first; loopback when it has no network. */
    private fun phoneAddresses(): List<String> {
        val found = NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback }
            .sortedByDescending { it.name.startsWith("wlan") }
            .flatMap { nic -> nic.inetAddresses.toList().filterIsInstance<Inet4Address>() }
            .mapNotNull { it.hostAddress }
        return found.ifEmpty { listOf("127.0.0.1") }
    }

    private object NoProgress : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        const val TAG = "BeamPairingTest"
    }
}
