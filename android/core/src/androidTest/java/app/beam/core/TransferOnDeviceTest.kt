package app.beam.core

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/** A loopback transfer between two engines inside the app process, on a real phone. */
@RunWith(AndroidJUnit4::class)
class TransferOnDeviceTest {
    private val root =
        File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "transfer-test")

    @After
    fun cleanUp() {
        root.deleteRecursively()
    }

    @Test
    fun aFileCrossesBetweenTwoEnginesWithProgress() = runBlocking {
        val source = File(root, "outbox/clip.bin")
        source.parentFile?.mkdirs()
        val bytes = ByteArray(64 * MIB) { (it * 31 % 253).toByte() }
        source.writeBytes(bytes)
        val inbox = File(root, "inbox").apply { mkdirs() }
        val phone = DeviceIdentity.generate()
        val received = AtomicLong()
        val progress =
            object : ProgressListener {
                override fun onProgress(bytesDone: ULong, bytesTotal: ULong) =
                    received.set(bytesDone.toLong())
            }

        Engine(DeviceIdentity.generate()).use { laptop ->
            Engine(phone).use { phoneEngine ->
                val listener = phoneEngine.listen(0u, SessionCipher.AES_GCM, NobodyTrusted)
                val outgoing = laptop.prepareSend(listOf(source.path))
                val receive =
                    async { listener.nextOffer().accept(inbox.path, emptyList(), progress) }
                val started = System.nanoTime()
                val result = outgoing.sendTo(
                    "127.0.0.1",
                    listener.port(),
                    phone.publicKey(),
                    SessionCipher.AES_GCM,
                    progress
                )
                receive.await()
                val seconds = (System.nanoTime() - started) / 1e9
                Log.i(
                    "BeamTransferTest",
                    "64 MiB loopback in %.2f s (%.0f MB/s)".format(
                        seconds,
                        64 * 1.048576 / seconds
                    )
                )
                assertEquals(SendResult.Delivered, result)
                assertEquals(64L * MIB, received.get())
                assertArrayEquals(bytes, File(inbox, "clip.bin").readBytes())
            }
        }
    }

    /** The laptop isn't paired: its offer still arrives, for the person to decide. */
    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private companion object {
        const val MIB = 1024 * 1024
    }
}
