package app.beam.core.lan

import android.Manifest
import android.os.Build
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.BeamException
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.TrustedKeys
import app.beam.core.discovery.BeaconAnnouncer
import app.beam.core.network.Hotspot
import app.beam.core.network.HotspotSession
import app.beam.core.network.LocalNetworks
import java.io.File
import java.net.InetAddress
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The phone and a `beam` CLI on a Mac, over a real network: the phone's Wi-Fi, or (with
 * `hotspot=true`) a hotspot the phone opens and the Mac joins. The phone waits for the Mac's
 * pairing code, pairs, sends the Mac a file, announces itself over mDNS (the Mac looks for
 * it), then receives a file from the Mac.
 *
 * Skipped unless `tools/mac-peer-test.sh` runs it (`macPeer=true`). It reads the `LAN ` lines
 * this test logs and pushes the code into [codeFile].
 */
@RunWith(AndroidJUnit4::class)
class MacPeerOnDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val root = File(context.cacheDir, "mac-peer-test")
    private val codeFile = File(context.getExternalFilesDir(null), "mac-code.txt")
    private val networks = LocalNetworks(context)
    private var hotspot: HotspotSession? = null
    private val announcing = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @After
    fun cleanUp() {
        announcing.cancel()
        hotspot?.close()
        networks.stop()
        root.deleteRecursively()
        codeFile.delete()
    }

    @Test
    fun pairSendAndReceiveWithTheMac() = runBlocking {
        val args = InstrumentationRegistry.getArguments()
        assumeTrue("run by tools/mac-peer-test.sh", args.getString("macPeer") == "true")
        val mib = args.getString("mib")?.toInt() ?: DEFAULT_MIB
        codeFile.delete()
        networks.start()
        if (args.getString("hotspot") == "true") openHotspot()

        say("READY")
        val (code, host, port) = withTimeout(MAC_WAIT_MS) { macCode() }
        Engine(DeviceIdentity.generate()).use { engine ->
            val mac = engine.pairWith(code, PHONE_NAME, null)
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

            val announcer = BeaconAnnouncer(context, engine, announcing)
            announcer.start(PHONE_PORT)
            val announced = withTimeout(ANNOUNCE_WAIT_MS) {
                announcer.state.first { it !is BeaconAnnouncer.State.Stopped }
            }
            check(announced is BeaconAnnouncer.State.Announcing) { "not announced: $announced" }
            say("ANNOUNCING beacon=${announced.beacon}")

            say("LISTENING port=$PHONE_PORT")
            val saved = withTimeout(RECEIVE_WAIT_MS) { receive.await() }
            assertEquals(1, saved.size)
            announcer.stop()
        }
    }

    private suspend fun openHotspot() {
        val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.NEARBY_WIFI_DEVICES
        } else {
            Manifest.permission.ACCESS_FINE_LOCATION
        }
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, permission)
        val opened = Hotspot(context).start().also { hotspot = it }
        // A throwaway network for this test only, so its password may be logged.
        say("HOTSPOT address=${opened.address?.hostAddress} security=${opened.security}")
        say("SSID ${opened.ssid}")
        say("PASS ${opened.passphrase.orEmpty()}")
    }

    /** The script pushes three lines: the code, the Mac's address and its receiving port. */
    private suspend fun macCode(): Triple<String, String, Int> {
        while (true) {
            val lines = codeFile.takeIf { it.exists() }?.readLines().orEmpty()
            if (lines.size >= 3) return Triple(lines[0], lines[1], lines[2].trim().toInt())
            delay(POLL_MS)
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
        const val MAC_WAIT_MS = 180_000L
        const val RECEIVE_WAIT_MS = 120_000L
        const val ANNOUNCE_WAIT_MS = 10_000L
        const val POLL_MS = 200L
        const val SEND_TRIES = 40
        const val SEND_RETRY_MS = 500L
    }
}
