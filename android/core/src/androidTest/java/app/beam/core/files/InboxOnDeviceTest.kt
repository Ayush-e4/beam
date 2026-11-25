package app.beam.core.files

import android.os.Build
import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.beam.core.DeviceIdentity
import app.beam.core.Engine
import app.beam.core.ProgressListener
import app.beam.core.SendResult
import app.beam.core.SessionCipher
import app.beam.core.TrustedKeys
import java.io.File
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Receiving into the real Download/Beam on a phone, and the gallery index seeing it. */
@RunWith(AndroidJUnit4::class)
class InboxOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val outbox = File(context.cacheDir, "inbox-test")
    private val saved = ArrayList<File>()

    @After
    fun cleanUp() {
        saved.forEach { it.delete() }
        outbox.deleteRecursively()
    }

    @Test
    fun receivedFilesLandInDownloadBeamAndAreIndexed() = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.R)
        assertEquals(InboxStatus.READY, Inbox.status(context))
        val stagedBefore = staged()
        val name = "beam-inbox-test-${System.nanoTime()}.bin"
        val bytes = ByteArray(4 * MIB) { (it * 13 % 241).toByte() }
        val source = File(outbox, name).apply {
            parentFile?.mkdirs()
            writeBytes(bytes)
        }

        // Twice: the second copy gets a free name instead of replacing the first.
        val first = receive(source)
        val second = receive(source)

        val folder = Inbox.folder(context)
        assertEquals(File(folder, name).path, first)
        assertTrue(second != first && File(second).parentFile == folder)
        assertArrayEquals(bytes, File(first).readBytes())
        assertArrayEquals(bytes, File(second).readBytes())
        assertTrue("indexed for the gallery and Files", indexed(name))
        Engine(DeviceIdentity.generate()).use { engine ->
            assertEquals(0u, Inbox.removeStale(engine, context)?.failed)
        }
        assertEquals("nothing unfinished left behind", stagedBefore, staged())
    }

    /** Unfinished transfers in Download/Beam (other tests may leave some on purpose). */
    private fun staged(): Set<String> =
        File(Inbox.folder(context), ".beam-partial").list()?.toSet() ?: emptySet()

    private suspend fun receive(source: File): String {
        val phone = DeviceIdentity.generate()
        return Engine(DeviceIdentity.generate()).use { laptop ->
            Engine(phone).use { phoneEngine ->
                val listener = phoneEngine.listen(0u, SessionCipher.AES_GCM, NobodyTrusted)
                val outgoing = laptop.prepareSend(listOf(source.path))
                runBlocking {
                    val receive = async {
                        val offer = listener.nextOffer()
                        assertEquals(false, offer.continuesInInbox(context))
                        offer.acceptIntoInbox(context, Ignore)
                    }
                    val result = outgoing.sendTo(
                        "127.0.0.1",
                        listener.port(),
                        phone.publicKey(),
                        SessionCipher.AES_GCM,
                        Ignore
                    )
                    assertEquals(SendResult.Delivered, result)
                    receive.await().single().also { saved += File(it) }
                }
            }
        }
    }

    private fun indexed(name: String): Boolean = context.contentResolver.query(
        MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL),
        arrayOf(MediaStore.MediaColumns.DISPLAY_NAME),
        "${MediaStore.MediaColumns.DISPLAY_NAME} = ? AND ${MediaStore.MediaColumns.RELATIVE_PATH} = ?",
        arrayOf(name, "${android.os.Environment.DIRECTORY_DOWNLOADS}/Beam/"),
        null
    )?.use { it.count > 0 } ?: false

    private object NobodyTrusted : TrustedKeys {
        override fun isTrusted(publicKey: ByteArray) = false
    }

    private object Ignore : ProgressListener {
        override fun onProgress(bytesDone: ULong, bytesTotal: ULong) = Unit
    }

    private companion object {
        const val MIB = 1024 * 1024
    }
}
