package app.beam.core.transfers

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.DeclineReason
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.OutgoingTransfer
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.SessionCipher
import app.beam.core.TrustedKeys
import app.beam.core.files.Inbox
import app.beam.core.history.Outcome
import app.beam.core.history.TransferHistory
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** The receive loop on a real phone: asking, declining, and continuing without asking. */
@RunWith(AndroidJUnit4::class)
class ReceivingOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val outbox = File(context.cacheDir, "receiving-test")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val registry = TransferRegistry()
    private val phone = DeviceIdentity.generate()
    private val phoneEngine = Engine(phone)
    private val laptopKey = DeviceIdentity.generate()
    private val laptop = Engine(laptopKey)
    private val runner = TransferRunner(scope, registry) {}
    private val finishedList = File(outbox, "finished-transfers.bin")
    private val history = TransferHistory(File(outbox, "transfer-history.bin"))
    private val receiving = receivingOn(phoneEngine)
    private val name = "beam-receiving-test-${System.nanoTime()}.bin"
    private val stem = name.substringBefore(".bin")

    @After
    fun cleanUp() {
        scope.cancel()
        laptop.close()
        phoneEngine.close()
        Inbox.folder(context).listFiles { file -> file.name.startsWith(stem) }
            ?.forEach { it.delete() }
        outbox.deleteRecursively()
    }

    @Test
    fun newOffersWaitForAnAnswer() = runBlocking {
        val bytes = ByteArray(MIB) { (it % 239).toByte() }
        val outgoing = laptop.prepareSend(listOf(source(bytes).path))
        val port = listen()

        val declined = async { send(outgoing, port) }
        val first = withTimeout(WAIT_MS) { receiving.offers.first { it.isNotEmpty() }.single() }
        assertEquals(name, first.summary.items.single().path)
        assertTrue(receiving.decline(first.id))
        assertEquals(SendResult.Declined(DeclineReason.USER_DECLINED), declined.await())
        val decline = history.all().single()
        assertEquals(Outcome.DECLINED to "USER_DECLINED", decline.outcome to decline.detail)
        assertEquals("Laptop", decline.deviceName)

        val delivered = async { send(outgoing, port) }
        val second = withTimeout(WAIT_MS) { receiving.offers.first { it.isNotEmpty() }.single() }
        val id = receiving.accept(second.id) ?: error("the offer stopped waiting")
        assertEquals(SendResult.Delivered, delivered.await())
        assertEquals(TransferState.Done, ended(id))
        assertEquals(emptyList<PendingOffer>(), receiving.offers.value)
        assertArrayEquals(bytes, File(Inbox.folder(context), name).readBytes())
        val entry = history.all().single()
        assertEquals("the same entry, now done", Outcome.DONE, entry.outcome)
        assertEquals(listOf(File(Inbox.folder(context), name).path), entry.savedPaths)
    }

    @Test
    fun anAcceptedTransferContinuesWithoutAsking() = runBlocking {
        val bytes = ByteArray(64 * MIB) { (it * 31 % 251).toByte() }
        val outgoing = laptop.prepareSend(listOf(source(bytes).path))
        val port = listen()

        val cut = async { send(outgoing, port) }
        val offer = withTimeout(WAIT_MS) { receiving.offers.first { it.isNotEmpty() }.single() }
        val first = receiving.accept(offer.id) ?: error("the offer stopped waiting")
        withTimeout(WAIT_MS) {
            registry.transfers.first { all -> all.any { it.id == first && it.bytesDone > 0 } }
        }
        registry.cancel(first)
        assertEquals(TransferState.Cancelled, ended(first))
        assertEquals(Outcome.CANCELLED, history.all().single().outcome)
        cut.await() // the laptop sees the link drop

        val result = send(outgoing, port)
        assertEquals(SendResult.Delivered, result)
        val second = registry.transfers.value.single { it.id != first }
        assertEquals(TransferState.Done, ended(second.id))
        assertEquals("never asked again", emptyList<PendingOffer>(), receiving.offers.value)
        assertArrayEquals(bytes, File(Inbox.folder(context), name).readBytes())
        assertEquals(Outcome.DONE, history.all().single().outcome)
    }

    @Test
    fun aRepeatAfterARestartIsConfirmedNotOffered() = runBlocking {
        val bytes = ByteArray(MIB) { (it % 233).toByte() }
        val outgoing = laptop.prepareSend(listOf(source(bytes).path))
        val run = scope.launch { receiving.run() }
        val port = withTimeout(WAIT_MS) { receiving.port.filterNotNull().first() }
        val delivered = async { send(outgoing, port) }
        val offer = withTimeout(WAIT_MS) { receiving.offers.first { it.isNotEmpty() }.single() }
        val id = receiving.accept(offer.id) ?: error("the offer stopped waiting")
        assertEquals(SendResult.Delivered, delivered.await())
        assertEquals(TransferState.Done, ended(id))
        run.cancel()
        run.join()

        // The app restarts with a new engine; the laptop missed "done" and asks again.
        Engine(phone).use { restarted ->
            val again = receivingOn(restarted)
            val rerun = scope.launch { again.run() }
            val newPort = withTimeout(WAIT_MS) { again.port.filterNotNull().first() }
            assertEquals(SendResult.Delivered, send(outgoing, newPort))
            assertEquals("never asked again", emptyList<PendingOffer>(), again.offers.value)
            rerun.cancel()
            rerun.join() // stop listening before the engine closes
        }
        val copies = Inbox.folder(context).listFiles { file -> file.name.startsWith(stem) }
        assertEquals("no second copy", 1, copies?.size)
    }

    @Test
    fun aDamagedListIsStartedAgain() = runBlocking {
        finishedList.parentFile?.mkdirs()
        finishedList.writeText("not a list")
        assertTrue("still listens", listen() > 0)
        Engine(phone).use { it.rememberFinishedIn(finishedList.path) } // a sound list now
    }

    private fun receivingOn(engine: Engine) = Receiving(
        context,
        engine,
        NobodyTrusted,
        runner,
        CIPHER,
        history,
        deviceName = { key -> "Laptop".takeIf { key.contentEquals(laptopKey.publicKey()) } },
        finishedList = finishedList
    )

    private fun source(bytes: ByteArray) = File(outbox, name).apply {
        parentFile?.mkdirs()
        writeBytes(bytes)
    }

    private suspend fun listen(): Int {
        scope.launch { receiving.run() }
        return withTimeout(WAIT_MS) { receiving.port.filterNotNull().first() }
    }

    private suspend fun send(outgoing: OutgoingTransfer, port: Int): SendResult? = try {
        outgoing.sendTo("127.0.0.1", port.toUShort(), phone.publicKey(), CIPHER, Ignore)
    } catch (e: app.beam.core.BeamException) {
        null
    }

    private suspend fun ended(id: Long): TransferState = withTimeout(WAIT_MS) {
        registry.transfers.first { all -> all.any { it.id == id && !it.isRunning } }
            .first { it.id == id }.state
    }

    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private object Ignore : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        const val MIB = 1024 * 1024
        const val WAIT_MS = 30_000L
        val CIPHER = SessionCipher.AES_GCM
    }
}
