package app.beam.core.clipboard

import app.beam.core.BeamException
import app.beam.core.ClipLinkInterface
import app.beam.core.NetworkFailure
import app.beam.core.ReceivedClip
import app.beam.core.SessionCipher
import app.beam.core.trust.DeviceKind
import app.beam.core.trust.TrustedDevice
import app.beam.core.trust.TrustedDevices
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class BrowserMeetingsTest {
    /** A link with the browser whose key is [key], until it is stopped. */
    private class FakeLink(private val key: ByteArray) : ClipLinkInterface {
        private val arrive = Channel<ReceivedClip>()

        override suspend fun nextClip(): ReceivedClip? = arrive.receiveCatching().getOrNull()

        override fun peerKey(): ByteArray = key

        override suspend fun sendText(text: String, sensitive: Boolean) = true

        override suspend fun sendImage(mime: String, bytes: ByteArray, sensitive: Boolean) = true

        override fun stop() {
            arrive.close()
        }
    }

    /** One wait in a browser's room, which the test ends by answering [link]. */
    private class Wait(val browser: TrustedDevice) {
        val link = CompletableDeferred<ClipLinkInterface>()
    }

    @get:Rule val folder = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val links = ClipLinks(scope, {}, {}, { _, _ -> })
    private val waits = Channel<Wait>(Channel.UNLIMITED)
    private val trusted by lazy {
        TrustedDevices(File(folder.root, "trusted.bin")).apply {
            add(device(1, "Pixel", DeviceKind.PHONE))
            add(device(2, "Chrome", DeviceKind.BROWSER))
            add(device(3, "Firefox", DeviceKind.BROWSER))
        }
    }
    private val meetings by lazy {
        BrowserMeetings(trusted, links, scope, meet = { browser ->
            Wait(browser).also { waits.send(it) }.link.await()
        }, retryMillis = 100, warn = { _, _ -> })
    }

    @After
    fun stop() = scope.cancel()

    private fun device(n: Int, name: String, kind: DeviceKind) = TrustedDevice(
        "id-$n",
        ByteArray(32) { n.toByte() },
        name,
        SessionCipher.CHA_CHA_POLY,
        ByteArray(32),
        n.toLong(),
        kind
    )

    private suspend fun nextWait() = withTimeout(5_000) { waits.receive() }

    @Test
    fun `it waits for each trusted browser while held, and links those that come`(): Unit =
        runBlocking {
            assertFalse(meetings.waiting.value)
            val held = meetings.hold()
            assertTrue(meetings.waiting.value)
            val names = setOf(nextWait(), nextWait()).associateBy { it.browser.name }
            assertEquals(setOf("Chrome", "Firefox"), names.keys)

            val chrome = names.getValue("Chrome")
            chrome.link.complete(FakeLink(chrome.browser.publicKey))
            val linked = withTimeout(5_000) { links.linked.first { it.isNotEmpty() } }
            assertEquals(listOf("Chrome"), linked.map { it.name })
            held.close()
        }

    @Test
    fun `a browser whose link ended is waited for again`(): Unit = runBlocking {
        val held = meetings.hold()
        val first = listOf(nextWait(), nextWait()).first { it.browser.name == "Chrome" }
        val link = FakeLink(first.browser.publicKey)
        first.link.complete(link)
        withTimeout(5_000) { links.linked.first { it.isNotEmpty() } }
        assertNull("no second wait while linked", withTimeoutOrNull(300) { waits.receive() })

        link.stop()
        assertEquals("Chrome", nextWait().browser.name)
        held.close()
    }

    @Test
    fun `a failed wait is tried again`(): Unit = runBlocking {
        val held = meetings.hold()
        val failed = nextWait()
        failed.link.completeExceptionally(BeamException.Network("no network", NetworkFailure.OTHER))
        val again = listOf(nextWait(), nextWait()).map { it.browser.name }
        assertTrue(again.toString(), failed.browser.name in again)
        held.close()
    }

    @Test
    fun `it stops when the last holder lets go`(): Unit = runBlocking {
        val app = meetings.hold()
        meetings.holdFor(300)
        listOf(nextWait(), nextWait())
        app.close()
        app.close() // closing twice lets go once
        assertTrue("the tile still holds it", meetings.waiting.value)
        delay(500)
        assertFalse(meetings.waiting.value)
        assertNull("nothing waits once stopped", withTimeoutOrNull(500) { waits.receive() })
    }
}
