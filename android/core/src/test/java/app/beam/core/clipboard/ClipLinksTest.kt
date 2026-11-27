package app.beam.core.clipboard

import app.beam.core.BeamException
import app.beam.core.ClipBody
import app.beam.core.ClipKind
import app.beam.core.ClipLinkInterface
import app.beam.core.NetworkFailure
import app.beam.core.ReceivedClip
import java.util.Collections
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipLinksTest {
    /** A link whose other device is the test: [arrive] is what it copies, [sent] what it got. */
    private class FakeLink(
        private val echoes: Set<String> = emptySet(),
        private val failSending: Boolean = false
    ) : ClipLinkInterface,
        AutoCloseable {
        val arrive = Channel<ReceivedClip>(Channel.UNLIMITED)
        val sent: MutableList<String> = Collections.synchronizedList(mutableListOf())

        @Volatile
        var stopped = false

        @Volatile
        var closed = false

        override suspend fun nextClip(): ReceivedClip? = arrive.receiveCatching().getOrNull()

        override fun peerKey(): ByteArray = ByteArray(32) { 7 }

        override suspend fun sendText(text: String, sensitive: Boolean): Boolean {
            if (failSending) throw BeamException.Network("gone", NetworkFailure.OTHER)
            if (text in echoes) return false
            sent += if (sensitive) "secret:$text" else text
            return true
        }

        override suspend fun sendImage(
            mime: String,
            bytes: ByteArray,
            sensitive: Boolean
        ): Boolean {
            sent += "$mime:${bytes.size}"
            return true
        }

        override fun stop() {
            stopped = true
            arrive.close()
        }

        override fun close() {
            closed = true
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val put = Channel<ReceivedClip>(Channel.UNLIMITED)
    private var linkedCalls = 0
    private val links = ClipLinks(scope, { put.trySend(it) }, { linkedCalls++ }, { _, _ -> })

    @After
    fun stop() = scope.cancel()

    private fun text(text: String) = ReceivedClip(ClipBody.Text(text), ClipKind.TEXT, false, null)

    private fun waitUntilLinked(count: Int) = runBlocking {
        withTimeout(5_000) { links.linked.first { it.size == count } }
    }

    @Test
    fun `what a linked device copies lands on the clipboard`() = runBlocking {
        val link = FakeLink()
        val device = links.add("Chrome on Mac", link)
        assertEquals(listOf(device), links.linked.value)
        assertEquals(1, linkedCalls)
        link.arrive.send(text("from the browser"))
        assertEquals(text("from the browser"), withTimeout(5_000) { put.receive() })
    }

    @Test
    fun `the clipboard goes to every linked device`() = runBlocking {
        val (a, b) = FakeLink() to FakeLink()
        links.add("A", a)
        links.add("B", b)
        val image = OutgoingClip.Image("image/png", ByteArray(3), sensitive = false)
        assertEquals(SendResult.SENT, links.send(OutgoingClip.Text("482913", sensitive = true)))
        assertEquals(SendResult.SENT, links.send(image))
        assertEquals(listOf("secret:482913", "image/png:3"), a.sent.toList())
        assertEquals(a.sent.toList(), b.sent.toList())
    }

    @Test
    fun `echoes, failures and empty reads say why nothing was sent`() = runBlocking {
        assertEquals(SendResult.NOBODY_LINKED, links.send(OutgoingClip.Text("hi", false)))
        links.add("Echo", FakeLink(echoes = setOf("hi")))
        assertEquals(SendResult.ALREADY_THERE, links.send(OutgoingClip.Text("hi", false)))
        assertEquals(SendResult.NOTHING_COPIED, links.send(ClipboardRead.Empty))
        assertEquals(SendResult.TOO_LARGE, links.send(ClipboardRead.TooLarge))
        assertEquals(SendResult.UNSUPPORTED, links.send(ClipboardRead.Unsupported))

        val failing = ClipLinks(scope, {}, {}, { _, _ -> })
        failing.add("Gone", FakeLink(failSending = true))
        assertEquals(SendResult.FAILED, failing.send(OutgoingClip.Text("hi", false)))
    }

    @Test
    fun `disconnecting ends the link and forgets the device`() {
        val (a, b) = FakeLink() to FakeLink()
        val first = links.add("A", a)
        links.add("B", b)
        links.disconnect(first)
        waitUntilLinked(1)
        assertTrue(a.stopped && a.closed)
        assertEquals("B", links.linked.value.single().name)
        links.disconnectAll()
        waitUntilLinked(0)
        assertTrue(b.stopped && b.closed)
    }

    @Test
    fun `a link that ends on its own is forgotten`() {
        val link = FakeLink()
        links.add("A", link)
        link.arrive.close()
        waitUntilLinked(0)
        assertTrue(link.closed)
    }
}
