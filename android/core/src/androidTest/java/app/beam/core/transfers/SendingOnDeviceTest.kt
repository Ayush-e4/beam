package app.beam.core.transfers

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.DeclineReason
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.SessionCipher
import app.beam.core.TrustedKeys
import app.beam.core.discovery.NearbyDevice
import app.beam.core.files.Inbox
import app.beam.core.trust.TrustedDevice
import java.io.File
import java.net.InetAddress
import java.security.MessageDigest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Sends that finish what they started, between two engines on a real phone. */
@RunWith(AndroidJUnit4::class)
class SendingOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val root = File(context.cacheDir, "sending-test")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val registry = TransferRegistry()
    private val runner = TransferRunner(scope, registry) {}
    private val phone = DeviceIdentity.generate()
    private val phoneEngine = Engine(phone)
    private val laptopEngine = Engine(DeviceIdentity.generate())
    private val receiving = Receiving(context, phoneEngine, NobodyTrusted, runner, CIPHER)
    private val pendingFile = File(root, "pending-sends.bin")
    private val name = "beam-sending-test-${System.nanoTime()}"
    private val device = TrustedDevice(
        phone.deviceId(),
        phone.publicKey(),
        "Phone",
        CIPHER,
        ByteArray(32),
        0
    )

    private val stagedBefore = staged()

    @After
    fun cleanUp() {
        scope.cancel()
        // Cancelled transfers keep their part for 7 days, as they should; not test leftovers.
        (staged() - stagedBefore.toSet()).forEach {
            File(File(Inbox.folder(context), ".beam-partial"), it).deleteRecursively()
        }
        laptopEngine.close()
        phoneEngine.close()
        Inbox.folder(context).listFiles { file -> file.name.startsWith(name) }
            ?.forEach { it.delete() }
        root.deleteRecursively()
    }

    @Test
    fun aDeliveredSendIsForgotten() = runBlocking {
        val bytes = ByteArray(MIB) { (it % 233).toByte() }
        val sending = sending()
        val id = sending.send(listen(), SourceKind.PATHS, listOf(source(bytes).path))
        assertEquals(1, PendingSends(pendingFile).all().size)

        receiving.accept(nextOffer().id)
        assertEquals(TransferState.Done, ended(id))
        assertEquals(emptyList<PendingSend>(), PendingSends(pendingFile).all())
        assertArrayEquals(bytes, File(Inbox.folder(context), "$name.bin").readBytes())
    }

    @Test
    fun aSendStoppedByAndroidResumesAfterARestart() = runBlocking {
        val source = bigSource()
        val nearby = listen()
        val first = sending().send(nearby, SourceKind.PATHS, listOf(source.path))
        receiving.accept(nextOffer().id)
        cutPartway(first) { registry.cancelAll(byPerson = false) } // Android stopped it
        assertEquals(TransferState.Cancelled, ended(first))
        assertEquals(1, PendingSends(pendingFile).all().size)
        assertTrue("part of it is on the phone", staged().isNotEmpty())

        // The app starts again and the phone is nearby: the send resumes by itself, and the
        // phone continues it without asking.
        sending().resume(listOf(nearby), listOf(device))
        val resumed = withTimeout(WAIT_MS) {
            registry.transfers.first { all ->
                all.any { it.direction == Direction.SENDING && it.id != first }
            }.first { it.direction == Direction.SENDING && it.id != first }
        }
        assertEquals(TransferState.Done, ended(resumed.id))
        assertEquals(emptyList<PendingOffer>(), receiving.offers.value)
        assertEquals(emptyList<PendingSend>(), PendingSends(pendingFile).all())
        assertArrayEquals(sha256(source), sha256(File(Inbox.folder(context), "$name.bin")))
    }

    @Test
    fun cancelledAndDeclinedSendsAreForgotten() = runBlocking {
        val nearby = listen()
        val big = bigSource().path
        val sending = sending()

        val cancelled = sending.send(nearby, SourceKind.PATHS, listOf(big))
        receiving.accept(nextOffer().id)
        cutPartway(cancelled) { registry.cancel(cancelled) }
        assertEquals(TransferState.Cancelled, ended(cancelled))
        assertEquals(emptyList<PendingSend>(), PendingSends(pendingFile).all())

        val declined = sending.send(nearby, SourceKind.PATHS, listOf(big))
        assertTrue(receiving.decline(nextOffer().id))
        assertEquals(TransferState.Declined(DeclineReason.USER_DECLINED), ended(declined))
        assertEquals(emptyList<PendingSend>(), PendingSends(pendingFile).all())
    }

    @Test
    fun oldSendsAndSendsToUntrustedDevicesAreForgotten() {
        val pending = PendingSends(pendingFile)
        val now = 100L * 24 * 60 * 60 * 1000
        val week = 7L * 24 * 60 * 60 * 1000
        fun send(id: String, deviceId: String, startedAt: Long) =
            PendingSend(id, deviceId, "x", SourceKind.PATHS, listOf("/nowhere"), startedAt)
        pending.add(send("old", device.deviceId, now - week - 1))
        pending.add(send("untrusted", "someone-else", now))
        pending.add(send("kept", device.deviceId, now - week + 60_000))

        // The phone isn't nearby, so nothing starts; only the two are forgotten.
        Sending(context, laptopEngine, runner, pending) { now }.resume(emptyList(), listOf(device))
        assertEquals(listOf("kept"), PendingSends(pendingFile).all().map { it.transferId })
    }

    private fun sending() = Sending(context, laptopEngine, runner, PendingSends(pendingFile))

    private fun source(bytes: ByteArray) = File(root, "$name.bin").apply {
        parentFile?.mkdirs()
        writeBytes(bytes)
    }

    /** A file too big to hold in memory, written a block at a time. */
    private fun bigSource() = File(root, "$name.bin").apply {
        parentFile?.mkdirs()
        val block = ByteArray(MIB)
        outputStream().use { out ->
            repeat(BIG / MIB) { n ->
                block.indices.forEach { block[it] = ((it * 7 + n) % 251).toByte() }
                out.write(block)
            }
        }
    }

    private fun sha256(file: File): ByteArray {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(MIB)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest()
    }

    private suspend fun listen(): NearbyDevice {
        scope.launch { receiving.run() }
        val port = withTimeout(WAIT_MS) { receiving.port.filterNotNull().first() }
        return NearbyDevice(device, listOf(InetAddress.getByName("127.0.0.1")), port, true)
    }

    private suspend fun nextOffer(): PendingOffer =
        withTimeout(WAIT_MS) { receiving.offers.first { it.isNotEmpty() }.first() }

    /**
     * Runs [cut] as soon as [id] has moved some bytes. Loopback is fast, so this watches from a
     * background thread rather than the test's, and the file is large enough to still be going.
     */
    private suspend fun cutPartway(id: Long, cut: () -> Unit) = withTimeout(WAIT_MS) {
        withContext(Dispatchers.Default) {
            registry.transfers.first { all -> all.any { it.id == id && it.bytesDone > 0 } }
            cut()
        }
    }

    /** Unfinished transfers on the phone. */
    private fun staged(): List<String> =
        File(Inbox.folder(context), ".beam-partial").list()?.toList() ?: emptyList()

    private suspend fun ended(id: Long): TransferState = withTimeout(WAIT_MS) {
        registry.transfers.first { all -> all.any { it.id == id && !it.isRunning } }
            .first { it.id == id }.state
    }

    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private companion object {
        const val MIB = 1024 * 1024
        const val BIG = 256 * MIB
        const val WAIT_MS = 60_000L
        val CIPHER = SessionCipher.AES_GCM
    }
}
