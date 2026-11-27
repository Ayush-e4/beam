package app.beam.core.clipboard

import android.Manifest
import android.app.NotificationManager
import android.content.ComponentName
import android.os.Build
import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.ClipBody
import app.beam.core.ClipKind
import app.beam.core.ReceivedClip
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The clipboard on a real phone: what a linked device copies lands on it, the invisible
 * focus window reads it back (Android 10+ allows that only with focus), and the link service
 * shows while a device is linked. The screen must be on and unlocked.
 */
@RunWith(AndroidJUnit4::class)
class ClipboardOnDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val links = ClipLinks.get(context)
    private val clipboard = PhoneClipboard(context)
    private val link = FakeLink()

    @Before
    fun link() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            instrumentation.uiAutomation.grantRuntimePermission(
                context.packageName,
                Manifest.permission.POST_NOTIFICATIONS
            )
        }
        links.add("Test browser", link)
    }

    @After
    fun unlink() {
        links.disconnectAll()
        runBlocking { withTimeout(TIMEOUT_MS) { links.linked.first { it.isEmpty() } } }
    }

    /** Opens the focus window, as the notification button does, and returns what it sent. */
    private fun sendClipboard(): Pair<String, Any> = runBlocking {
        context.startActivity(ClipFocusActivity.intent(context))
        withTimeout(TIMEOUT_MS) { link.sent.receive() }
    }

    @Test
    fun aCopyFromTheBrowserLandsOnTheClipboardAndGoesBack() = runBlocking {
        clipboard.put(ReceivedClip(ClipBody.Text("before"), ClipKind.TEXT, false, null))
        val clip = ReceivedClip(ClipBody.Text("from the browser"), ClipKind.TEXT, false, null)
        link.arrive.send(clip)
        // Only a focused window can read the clipboard, so look by sending until it changed.
        withTimeout(TIMEOUT_MS) {
            while (sendClipboard() != ("text" to "from the browser")) delay(100)
        }
    }

    @Test
    fun codesAndSensitiveTextGoAsSensitive() {
        clipboard.put(ReceivedClip(ClipBody.Text("hunter2"), ClipKind.TEXT, true, null))
        assertEquals("secret" to "hunter2", sendClipboard())
        clipboard.put(ReceivedClip(ClipBody.Text("G-482913"), ClipKind.TEXT, false, null))
        assertEquals("secret" to "G-482913", sendClipboard())
    }

    @Test
    fun imagesGoWhole() {
        val png = ByteArray(200 * 1024) { (it % 251).toByte() }
        clipboard.put(ReceivedClip(ClipBody.Image("image/png", png), ClipKind.IMAGE, false, null))
        val (mime, bytes) = sendClipboard()
        assertEquals("image/png", mime)
        assertArrayEquals(png, bytes as ByteArray)
    }

    @Test
    fun theTileSendsTheClipboard() {
        clipboard.put(
            ReceivedClip(ClipBody.Text("tapped from the tile"), ClipKind.TEXT, false, null)
        )
        val tile = ComponentName(context, ClipTileService::class.java).flattenToString()
        shell("cmd statusbar add-tile $tile")
        try {
            shell("cmd statusbar click-tile $tile")
            val sent = runBlocking { withTimeout(TIMEOUT_MS) { link.sent.receive() } }
            assertEquals("text" to "tapped from the tile", sent)
        } finally {
            shell("cmd statusbar remove-tile $tile")
            shell("cmd statusbar collapse")
        }
    }

    /** Runs a shell command as the test's shell user and waits for it to finish. */
    private fun shell(command: String) {
        instrumentation.uiAutomation.executeShellCommand(command).use { output ->
            ParcelFileDescriptor.AutoCloseInputStream(output).use { it.readBytes() }
        }
    }

    @Test
    fun theLinkServiceShowsWhileADeviceIsLinked() = runBlocking {
        val manager = context.getSystemService(NotificationManager::class.java)
        withTimeout(TIMEOUT_MS) {
            while (manager.activeNotifications.none { it.id == NOTIFICATION_ID }) {
                delay(50)
            }
        }
        val shown = manager.activeNotifications.first { it.id == NOTIFICATION_ID }.notification
        assertTrue("it can send the clipboard and disconnect", shown.actions.size == 2)
    }

    private companion object {
        const val TIMEOUT_MS = 10_000L
        const val NOTIFICATION_ID = 3
    }
}
