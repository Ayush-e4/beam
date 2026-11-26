package app.beam.core.network

import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.net.Inet4Address
import java.net.InetAddress
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/** On a real phone on Wi-Fi: nearby addresses map to the Wi-Fi network, others don't. */
@RunWith(AndroidJUnit4::class)
class LocalNetworksOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val networks = LocalNetworks(context)

    @After
    fun stop() = networks.stop()

    @Test
    fun addressesOnTheWifiSubnetGoOverWifi() = runBlocking {
        val wifi = connectivity.activeNetwork?.takeIf {
            connectivity.getNetworkCapabilities(it)
                ?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        }
        assumeTrue("the phone is on Wi-Fi", wifi != null)
        val own = connectivity.getLinkProperties(wifi)?.linkAddresses
            ?.firstOrNull { it.address is Inet4Address && it.prefixLength in 8..30 }
        assumeTrue("its Wi-Fi has an IPv4 subnet", own != null)
        val ownAddress = requireNotNull(own).address
        val neighbour = InetAddress.getByAddress(
            ownAddress.address.copyOf().also { it[3] = (it[3].toInt() xor 1).toByte() }
        )

        networks.start()
        val expected = requireNotNull(wifi).networkHandle
        withTimeout(WAIT_MS) {
            while (networks.networkFor(ownAddress) == null) delay(POLL_MS)
        }
        assertEquals(expected, networks.networkFor(ownAddress))
        assertEquals("a nearby device", expected, networks.networkFor(neighbour))
        assertNull("the internet", networks.networkFor(InetAddress.getByName("8.8.8.8")))
        assertNull("this phone", networks.networkFor(InetAddress.getByName("127.0.0.1")))

        networks.stop()
        assertNull("forgotten when stopped", networks.networkFor(ownAddress))
    }

    private companion object {
        const val WAIT_MS = 5_000L
        const val POLL_MS = 50L
    }
}
