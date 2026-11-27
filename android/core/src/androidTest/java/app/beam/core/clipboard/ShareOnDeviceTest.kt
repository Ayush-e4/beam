package app.beam.core.clipboard

import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/** "Share → Beam" on a real phone: shared text and images reach the linked device. */
@RunWith(AndroidJUnit4::class)
class ShareOnDeviceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val links = ClipLinks.get(context)
    private val link = FakeLink()
    private var shared: Uri? = null

    @Before
    fun link() {
        links.add("Test browser", link)
    }

    @After
    fun cleanUp() {
        links.disconnectAll()
        runBlocking { withTimeout(TIMEOUT_MS) { links.linked.first { it.isEmpty() } } }
        shared?.let { context.contentResolver.delete(it, null, null) }
    }

    private fun share(intent: Intent): Pair<String, Any> = runBlocking {
        context.startActivity(
            intent.setClass(context, ShareToBeamActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        )
        withTimeout(TIMEOUT_MS) { link.sent.receive() }
    }

    private fun text(text: String) =
        Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)

    @Test
    fun sharedLinksAndCodesReachTheLinkedDevice() {
        assertEquals("text" to "https://beam.app/x", share(text("https://beam.app/x")))
        assertEquals("secret" to "482913", share(text("482913")))
    }

    @Test
    fun aSharedPhotoGoesWhole() {
        val png = ByteArray(150 * 1024) { (it % 239).toByte() }
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "beam-share-test.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
        }
        val uri =
            checkNotNull(resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values))
        shared = uri
        checkNotNull(resolver.openOutputStream(uri)).use { it.write(png) }
        val intent = Intent(Intent.ACTION_SEND).setType("image/png")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        val (mime, bytes) = share(intent)
        assertEquals("image/png", mime)
        assertArrayEquals(png, bytes as ByteArray)
    }

    @Test
    fun beamsOwnFilesAreNeverSent() {
        val private = File(context.filesDir, "secret.png").apply { writeBytes(byteArrayOf(1, 2)) }
        val own = ClipImages(context).save("image/png", byteArrayOf(3, 4))
        for (uri in listOf(Uri.fromFile(private), own)) {
            val intent = Intent(Intent.ACTION_SEND).setType("image/png")
                .putExtra(Intent.EXTRA_STREAM, uri)
            assertEquals("$uri", ClipboardRead.Unsupported, SharedClip.read(context, intent))
        }
        private.delete()
    }

    private companion object {
        const val TIMEOUT_MS = 10_000L
    }
}
