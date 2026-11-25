package app.beam.core

import java.io.File
import java.util.Collections
import kotlin.random.Random
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Two engines on loopback, driven through the generated Kotlin API. */
class TransferTest {
    @get:Rule val folder = TemporaryFolder()

    private class Recorder(private val threshold: Long = Long.MAX_VALUE) : ProgressListener {
        val reports: MutableList<Pair<Long, Long>> = Collections.synchronizedList(mutableListOf())
        val passed = CompletableDeferred<Unit>()

        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) {
            reports += bytesDone.toLong() to bytesTotal.toLong()
            if (bytesDone.toLong() >= threshold) passed.complete(Unit)
        }
    }

    private val laptop = DeviceIdentity.generate()
    private val phone = DeviceIdentity.generate()

    private fun album(bigFile: Int): File {
        val album = File(folder.root, "album").apply { mkdirs() }
        // Incompressible: the receiver checkpoints by wire bytes, and the resume test needs
        // checkpoints to happen as the file arrives.
        File(album, "video.bin").writeBytes(Random(7).nextBytes(bigFile))
        repeat(20) { File(album, "note-$it.txt").writeText("note $it") }
        return album
    }

    private fun inbox() = File(folder.root, "inbox").apply { mkdirs() }

    @Test
    fun `a folder arrives with progress`() = runBlocking {
        val source = album(5 * MIB)
        Engine(laptop).use { laptopEngine ->
            Engine(phone).use { phoneEngine ->
                val listener = phoneEngine.listen(0u, CIPHER)
                val outgoing = laptopEngine.prepareSend(listOf(source.path))
                val total = outgoing.summary().totalBytes.toLong()
                val received = Recorder()
                val receive =
                    async {
                        val offer = listener.nextOffer()
                        assertEquals(laptop.deviceId(), offer.peerDeviceId())
                        offer.accept(inbox().path, emptyList(), received)
                    }
                val result = outgoing.sendTo(
                    "127.0.0.1",
                    listener.port(),
                    phone.publicKey(),
                    CIPHER,
                    Recorder()
                )
                assertEquals(SendResult.Delivered, result)
                assertEquals(21, receive.await().size)
                assertEquals(total to total, received.reports.last())
                assertSame(source, File(inbox(), "album"))
            }
        }
    }

    @Test
    fun `cancelling a receive coroutine stops it and the next attempt resumes`() = runBlocking {
        val source = album(64 * MIB)
        Engine(laptop).use { laptopEngine ->
            Engine(phone).use { phoneEngine ->
                val listener = phoneEngine.listen(0u, CIPHER)
                val outgoing = laptopEngine.prepareSend(listOf(source.path))
                val port = listener.port()

                val partial = Recorder(threshold = 8L * MIB)
                val receive =
                    async { listener.nextOffer().accept(inbox().path, emptyList(), partial) }
                val send =
                    async {
                        runCatching {
                            outgoing.sendTo(
                                "127.0.0.1",
                                port,
                                phone.publicKey(),
                                CIPHER,
                                Recorder()
                            )
                        }
                    }
                withTimeout(30_000) { partial.passed.await() }
                receive.cancel()
                val first = withTimeout(30_000) { send.await() }
                assertTrue(
                    "the sender saw the link drop",
                    first.exceptionOrNull() is BeamException.Network
                )

                val resumed = Recorder()
                val again =
                    async { listener.nextOffer().accept(inbox().path, emptyList(), resumed) }
                val result = outgoing.sendTo(
                    "127.0.0.1",
                    port,
                    phone.publicKey(),
                    CIPHER,
                    Recorder()
                )
                again.await()
                assertEquals(SendResult.Delivered, result)
                assertTrue(
                    "resumed from ${resumed.reports.first()}",
                    resumed.reports.first().first > 0
                )
                assertSame(source, File(inbox(), "album"))
            }
        }
    }

    private fun assertSame(source: File, received: File) {
        source.walkTopDown().filter { it.isFile }.forEach { file ->
            val there = File(received, file.relativeTo(source).path)
            assertArrayEquals(file.path, file.readBytes(), there.readBytes())
        }
    }

    private companion object {
        const val MIB = 1024 * 1024
        val CIPHER = SessionCipher.CHA_CHA_POLY
    }
}
