package app.beam.core.files

import android.content.ContentValues
import android.net.Uri
import android.os.Build
import android.os.ParcelFileDescriptor
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
import java.io.IOException
import kotlin.concurrent.thread
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Content URIs, as the gallery and share sheet hand them over, sent on a real phone. */
@RunWith(AndroidJUnit4::class)
class SharedFilesOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val root = File(context.cacheDir, "shared-test")
    private val inserted = ArrayList<Uri>()

    @After
    fun cleanUp() {
        inserted.forEach { context.contentResolver.delete(it, null, null) }
        root.deleteRecursively()
    }

    @Test
    fun aMediaStoreItemIsSentUnderItsDisplayName() = runBlocking {
        assumeTrue(Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q)
        val bytes = ByteArray(5 * MIB) { (it * 7 % 251).toByte() }
        val download = insertDownload("beam-shared-test-${System.nanoTime()}.bin", bytes)
        val note = File(root, "outbox/note.txt").apply {
            parentFile?.mkdirs()
            writeText("from a file URI")
        }
        val inbox = File(root, "inbox").apply { mkdirs() }

        val name = displayName(context.contentResolver, download)
        val result = sendAndReceive(listOf(download, Uri.fromFile(note)), inbox)
        assertEquals(SendResult.Delivered, result)
        assertArrayEquals(bytes, File(inbox, name).readBytes())
        assertEquals("from a file URI", File(inbox, "note.txt").readText())
    }

    @Test
    fun aStreamedSourceIsCopiedAndTheCopyLeavesNoFileBehind() {
        val bytes = ByteArray(3 * MIB) { (it % 249).toByte() }
        val (read, write) = ParcelFileDescriptor.createPipe()
        val writer = thread {
            ParcelFileDescriptor.AutoCloseOutputStream(write).use { it.write(bytes) }
        }
        val spill = File(root, "spill")
        read.use { pipe ->
            readableAtAnyPosition(pipe, spill).use { copy ->
                writer.join()
                assertTrue(copy !== pipe)
                val copied = ParcelFileDescriptor.AutoCloseInputStream(copy.dup()).readBytes()
                assertArrayEquals(bytes, copied)
            }
        }
        assertEquals(emptyList<String>(), spill.list()?.toList())
    }

    @Test(expected = IOException::class)
    fun aMissingItemIsAnIoError() {
        runBlocking {
            Engine(DeviceIdentity.generate()).use { engine ->
                engine.prepareSendUris(context, listOf(Uri.fromFile(File(root, "gone.bin"))))
            }
        }
    }

    private suspend fun sendAndReceive(uris: List<Uri>, inbox: File): SendResult {
        val phone = DeviceIdentity.generate()
        return Engine(DeviceIdentity.generate()).use { laptop ->
            Engine(phone).use { phoneEngine ->
                val listener = phoneEngine.listen(0u, SessionCipher.AES_GCM, NobodyTrusted)
                val outgoing = laptop.prepareSendUris(context, uris)
                runBlocking {
                    val receive =
                        async { listener.nextOffer().accept(inbox.path, emptyList(), Ignore) }
                    val result = outgoing.sendTo(
                        "127.0.0.1",
                        listener.port(),
                        phone.publicKey(),
                        SessionCipher.AES_GCM,
                        Ignore
                    )
                    receive.await()
                    result
                }
            }
        }
    }

    private fun insertDownload(name: String, bytes: ByteArray): Uri {
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: throw IOException("MediaStore refused $name")
        inserted += uri
        resolver.openOutputStream(uri)?.use { it.write(bytes) }
            ?: throw IOException("could not write $name")
        resolver.update(
            uri,
            ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
            null,
            null
        )
        return uri
    }

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
