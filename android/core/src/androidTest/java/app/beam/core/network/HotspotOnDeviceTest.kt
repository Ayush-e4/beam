package app.beam.core.network

import android.Manifest
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.SessionCipher
import app.beam.core.TrustedKeys
import java.io.File
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/** The phone's own hotspot, on a real phone: it opens, says how to join, and carries a transfer. */
@RunWith(AndroidJUnit4::class)
class HotspotOnDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val root = File(context.cacheDir, "hotspot-test")
    private val hotspot = Hotspot(context)
    private var session: HotspotSession? = null

    @Before
    fun allowNearbyWifi() {
        val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.NEARBY_WIFI_DEVICES
        } else {
            Manifest.permission.ACCESS_FINE_LOCATION
        }
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, permission)
    }

    @After
    fun cleanUp() {
        session?.close()
        root.deleteRecursively()
    }

    @Test
    fun theHotspotOpensAndCarriesATransfer() = runBlocking {
        val wifiBefore = onWifi()
        val opened = open()
        Log.i(TAG, "${opened.security} at ${opened.address}; on Wi-Fi: $wifiBefore -> ${onWifi()}")
        assertTrue(opened.ssid.isNotBlank())
        if (opened.security != WifiSecurity.OPEN) assertTrue(!opened.passphrase.isNullOrEmpty())
        assertTrue(opened.wifiQr.startsWith("WIFI:T:"))
        assertTrue(opened.isOn.value)
        val address = requireNotNull(opened.address?.hostAddress) { "no address on the hotspot" }

        // A device on the hotspot reaches the phone at that address.
        val phone = DeviceIdentity.generate()
        Engine(phone).use { phoneEngine ->
            Engine(DeviceIdentity.generate()).use { laptop ->
                val inbox = File(root, "inbox").apply { mkdirs() }
                val listener = phoneEngine.listen(0u, CIPHER, NobodyTrusted)
                val receive = async { listener.nextOffer().accept(inbox.path, emptyList(), Quiet) }
                val sent = laptop.prepareSend(listOf(source().path)).sendTo(
                    address,
                    listener.port(),
                    phone.publicKey(),
                    CIPHER,
                    Quiet
                )
                receive.await()
                assertEquals(SendResult.Delivered, sent)
                assertEquals("over the hotspot", File(inbox, "note.txt").readText())
            }
        }

        opened.close()
        assertFalse(opened.isOn.value)
    }

    private suspend fun open(): HotspotSession {
        val opened = try {
            hotspot.start()
        } catch (busy: HotspotException) {
            // The phone's own hotspot (tethering) being on rules this one out.
            assumeTrue("the radio is free", busy.problem != HotspotProblem.INCOMPATIBLE_MODE)
            throw busy
        }
        session = opened
        return opened
    }

    private fun onWifi() = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
        ?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true

    private fun source() = File(root, "outbox/note.txt").apply {
        parentFile?.mkdirs()
        writeText("over the hotspot")
    }

    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private object Quiet : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        const val TAG = "BeamHotspotTest"
        val CIPHER = SessionCipher.AES_GCM
    }
}
