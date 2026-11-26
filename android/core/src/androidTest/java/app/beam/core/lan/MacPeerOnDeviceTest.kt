package app.beam.core.lan

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.BeamException
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.TrustedKeys
import app.beam.core.network.LocalNetworks
import java.io.File
import java.net.InetAddress
import kotlin.random.Random
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The phone and a `beam` CLI on a Mac, over the real Wi-Fi: pair with the Mac's code, send it
 * a file, then receive one from it. Skipped unless `tools/lan-test.sh` passes the arguments
 * (`macCode`, `macHost`, `macPort`, `mib`); it reads the `LAN ` lines this test logs.
 */
@RunWith(AndroidJUnit4::class)
class MacPeerOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val root = File(context.cacheDir, "mac-peer-test")
    private val networks = LocalNetworks(context)

    @After
    fun cleanUp() {
        networks.stop()
        root.deleteRecursively()
    }

    @Test
    fun pairSendAndReceiveWithTheMac() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        val code = args.getString("macCode")
        assumeTrue("run by tools/lan-test.sh", code != null)
        val host = requireNotNull(args.getString("macHost"))
        val port = requireNotNull(args.getString("macPort")).toInt()
        val mib = args.getString("mib")?.toInt() ?: DEFAULT_MIB
        networks.start()

        Engine(DeviceIdentity.generate()).use { engine ->
            val mac = engine.pairWith(requireNotNull(code), PHONE_NAME, null)
            say("PAIRED verify=${mac.verificationCode}")

            val listener = engine.listen(PHONE_PORT.toUShort(), mac.cipher, Only(mac.publicKey))
            val inbox = File(root, "inbox").apply { mkdirs() }
            val receive = async {
                val offer = listener.nextOffer()
                val started = System.nanoTime()
                val saved = offer.accept(inbox.path, emptyList(), Quiet)
                say("RECEIVED files=${saved.size} ${rate(offer.summary().totalBytes, started)}")
                saved
            }

            val file = bigFile(mib)
            val outgoing = engine.prepareSend(listOf(file.path))
            val network = networks.networkFor(InetAddress.getByName(host))?.toULong()
            say("SENDING bytes=${file.length()}")
            val started = System.nanoTime()
            val sent = untilListening {
                outgoing.sendTo(host, port.toUShort(), mac.publicKey, mac.cipher, Quiet, network)
            }
            assertEquals(SendResult.Delivered, sent)
            say("SENT ${rate(file.length().toULong(), started)}")

            say("LISTENING port=$PHONE_PORT")
            val saved = withTimeout(RECEIVE_WAIT_MS) { receive.await() }
            assertEquals(1, saved.size)
        }
    }

    /** The Mac starts receiving only once pairing ends, so the first tries may be refused. */
    private suspend fun <T> untilListening(send: suspend () -> T): T {
        repeat(SEND_TRIES - 1) {
            try {
                return send()
            } catch (refused: BeamException.Network) {
                Log.i(TAG, "Mac not receiving yet: ${refused.message}")
                delay(SEND_RETRY_MS)
            }
        }
        return send()
    }

    /** Random bytes, so compression can't flatter the speed. */
    private fun bigFile(mib: Int) = File(root, "from-phone.bin").apply {
        parentFile?.mkdirs()
        val random = Random(mib)
        val block = ByteArray(MIB)
        outputStream().use { out ->
            repeat(mib) {
                random.nextBytes(block)
                out.write(block)
            }
        }
    }

    private fun rate(bytes: ULong, startedNanos: Long): String {
        val seconds = (System.nanoTime() - startedNanos) / 1e9
        return "bytes=$bytes secs=%.2f mbps=%.0f".format(seconds, bytes.toDouble() / MIB / seconds)
    }

    private fun say(line: String) = Log.i(TAG, "LAN $line")

    private class Only(private val key: ByteArray) : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = publicKey.contentEquals(key)
    }

    private object Quiet : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        const val TAG = "BeamMacPeer"
        const val PHONE_NAME = "Phone under test"
        const val PHONE_PORT = 47124
        const val MIB = 1024 * 1024
        const val DEFAULT_MIB = 64
        const val RECEIVE_WAIT_MS = 120_000L
        const val SEND_TRIES = 40
        const val SEND_RETRY_MS = 500L
    }
}
