package app.beam.core

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.net.Inet4Address
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Sending over a chosen Android network (how Beam gets past a VPN), on a real phone on Wi-Fi:
 * the engine hands the socket to Android with that network's handle.
 */
@RunWith(AndroidJUnit4::class)
class NetworkChoiceOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val root = File(context.cacheDir, "network-choice-test")
    private val phone = DeviceIdentity.generate()
    private val phoneEngine = Engine(phone)
    private val laptop = Engine(DeviceIdentity.generate())

    @After
    fun cleanUp() {
        laptop.close()
        phoneEngine.close()
        root.deleteRecursively()
    }

    @Test
    fun aSendOverTheWifiNetworkArrives() = runBlocking {
        val wifi = wifiNetwork()
        val address = connectivity.getLinkProperties(wifi)?.linkAddresses
            ?.map { it.address }
            ?.firstOrNull { it is Inet4Address }
            ?.hostAddress
        assumeTrue("the phone's Wi-Fi has an IPv4 address", address != null)
        val inbox = File(root, "inbox").apply { mkdirs() }
        val listener = phoneEngine.listen(0u, CIPHER, NobodyTrusted)
        val receive = async { listener.nextOffer().accept(inbox.path, emptyList(), Quiet) }

        val result = laptop.prepareSend(listOf(source().path)).sendTo(
            address ?: return@runBlocking,
            listener.port(),
            phone.publicKey(),
            CIPHER,
            Quiet,
            wifi.networkHandle.toULong()
        )
        receive.await()
        assertEquals(SendResult.Delivered, result)
        assertEquals("over Wi-Fi", File(inbox, "note.txt").readText())
    }

    @Test
    fun aNetworkAndroidDoesntKnowIsANetworkError() {
        val listener = runBlocking { phoneEngine.listen(0u, CIPHER, NobodyTrusted) }
        val outgoing = runBlocking { laptop.prepareSend(listOf(source().path)) }
        assertThrows(BeamException.Network::class.java) {
            runBlocking {
                outgoing.sendTo(
                    "127.0.0.1",
                    listener.port(),
                    phone.publicKey(),
                    CIPHER,
                    Quiet,
                    NO_SUCH_NETWORK
                )
            }
        }
    }

    @Test
    fun aPortNothingListensOnIsRefused() {
        val closed = java.net.ServerSocket(0).use { it.localPort }
        val outgoing = runBlocking { laptop.prepareSend(listOf(source().path)) }
        val error = assertThrows(BeamException.Network::class.java) {
            runBlocking {
                outgoing.sendTo("127.0.0.1", closed.toUShort(), phone.publicKey(), CIPHER, Quiet)
            }
        }
        assertEquals(NetworkFailure.REFUSED, error.failure)
    }

    private fun wifiNetwork(): Network {
        val network = connectivity.activeNetwork?.takeIf { network ->
            connectivity.getNetworkCapabilities(network)
                ?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        }
        assumeTrue("the phone is on Wi-Fi", network != null)
        return requireNotNull(network)
    }

    private fun source() = File(root, "outbox/note.txt").apply {
        parentFile?.mkdirs()
        writeText("over Wi-Fi")
    }

    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private object Quiet : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        val CIPHER = SessionCipher.CHA_CHA_POLY

        /** Network handles are Android's own numbers; this one names no network. */
        const val NO_SUCH_NETWORK = 0x7777_0000_0000_0001uL
    }
}
