package app.beam.core.clipboard

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.ClipBody
import app.beam.core.ClipKind
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.ReceivedClip
import app.beam.core.trust.TrustedDevices
import java.io.File
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The phone's half of the real clipboard test with Chromium on a Mac over Wi-Fi
 * (`tools/phone-browser-test.sh` runs both). It links with the browser showing the code it
 * is given, logs the offer for the Mac to hand over (standing in for signaling), takes the
 * browser's text and image, then sends the clipboard back through the focus window. Skipped
 * unless run with `-e browserCode BEAM1:…`. The screen must be on and unlocked.
 */
@RunWith(AndroidJUnit4::class)
class BrowserLinkOnDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    @Test
    fun linksWithChromiumOnAMac(): Unit = runBlocking {
        val code = InstrumentationRegistry.getArguments().getString("browserCode")
        assumeTrue("run by tools/phone-browser-test.sh", code != null)
        val root = File(context.cacheDir, "browser-link-test").apply { mkdirs() }
        val links = ClipLinks.get(context)
        Engine(DeviceIdentity.generate()).use { engine ->
            val trusted = TrustedDevices(File(root, "trusted-devices.bin"))
            val pending = BrowserLinking(engine, links, trusted).start(checkNotNull(code))
            Log.i(TAG, "OFFER ${pending.offer.toHex()}")
            val arrivals = async { links.arrivals.take(2).toList() }
            val browser = withTimeout(STEP_MS) { pending.await("Galaxy S21 FE") }
            Log.i(TAG, "LINKED ${browser.name}")
            assertNotNull("kept as trusted", trusted.find(browser.publicKey))

            val (text, image) = withTimeout(STEP_MS) { arrivals.await() }.map { it.clip }
            assertEquals(ClipBody.Text("from the Mac"), text.body)
            val png = image.body as ClipBody.Image
            assertEquals("image/png" to IMAGE_LEN, png.mime to png.bytes.size)
            Log.i(TAG, "GOT text and a ${png.bytes.size} byte image")

            for (copied in listOf("from the phone", "Your OTP is 482913")) {
                PhoneClipboard(
                    context
                ).put(ReceivedClip(ClipBody.Text(copied), ClipKind.TEXT, false, null))
                context.startActivity(ClipFocusActivity.intent(context))
                Log.i(TAG, "SENT $copied")
                delay(SEND_GAP_MS) // one focus window at a time
            }
            withTimeout(STEP_MS) { links.linked.first { it.isEmpty() } }
            Log.i(TAG, "ENDED")
        }
        root.deleteRecursively()
    }

    private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }

    private companion object {
        const val TAG = "BeamE2E"
        const val STEP_MS = 60_000L
        const val SEND_GAP_MS = 1_500L
        const val IMAGE_LEN = 300 * 1024
    }
}
