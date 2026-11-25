package app.beam.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.discovery.BeaconAnnouncer
import app.beam.core.discovery.NearbyBrowser
import app.beam.core.discovery.NearbyDevice
import app.beam.core.trust.TrustedDevice
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** One engine announces over the phone's real mDNS stack; a second that trusts it finds it. */
@RunWith(AndroidJUnit4::class)
class DiscoveryOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun aTrustedDeviceIsFoundBeforeAndAfterItsBeaconChanges() = runBlocking {
        val laptop = DeviceIdentity.generate()
        Engine(laptop).use { laptopEngine ->
            Engine(DeviceIdentity.generate()).use { phoneEngine ->
                val trusted = pair(laptopEngine, phoneEngine)
                val scope = CoroutineScope(Dispatchers.Default)
                val announcer = BeaconAnnouncer(context, laptopEngine, scope)
                val browser = NearbyBrowser(context) { listOf(trusted) }
                try {
                    announcer.start(PORT)
                    val first = announcer.announced()
                    val found = browser.nearby().firstMatching { it.port == PORT }
                    assertEquals(laptop.deviceId(), found.device.deviceId)
                    assertTrue("speaks our protocol", found.compatible)
                    assertTrue("has an address", found.addresses.isNotEmpty())

                    announcer.rotate()
                    val second = announcer.announced(not = first)
                    assertNotEquals(first, second)
                    val again = browser.nearby().firstMatching { it.port == PORT }
                    assertEquals(laptop.deviceId(), again.device.deviceId)
                } finally {
                    announcer.stop()
                    scope.cancel()
                }
            }
        }
    }

    private suspend fun pair(host: Engine, scanner: Engine): TrustedDevice = kotlinx.coroutines
        .coroutineScope {
            val pairing = host.hostPairing(SessionCipher.AES_GCM)
            val code = pairing.code(listOf("127.0.0.1"))
            val shown = async { pairing.waitForDevice("Laptop") }
            val scanned = scanner.pairWith(code, "Phone")
            shown.await()
            TrustedDevice.from(scanned, System.currentTimeMillis())
        }

    private suspend fun BeaconAnnouncer.announced(not: String? = null): String =
        withTimeout(TIMEOUT_MS) {
            state.filterIsInstance<BeaconAnnouncer.State.Announcing>()
                .first { it.beacon != not }
                .beacon
        }

    private suspend fun Flow<List<NearbyDevice>>.firstMatching(
        predicate: (NearbyDevice) -> Boolean
    ): NearbyDevice = withTimeout(TIMEOUT_MS) {
        first { list -> list.any(predicate) }.first(predicate)
    }

    private companion object {
        const val PORT = 47_123
        const val TIMEOUT_MS = 30_000L
    }
}
