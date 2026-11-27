package app.beam.core.clipboard

import android.content.Context
import android.net.Uri
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream

/**
 * Images from linked devices, saved where the clipboard can point at them: a content URI
 * other apps may read when they paste (`FileProvider`, see `res/xml/beam_clip_paths.xml`).
 * Only the newest few are kept.
 */
internal class ClipImages(private val context: Context) {
    private val dir = File(context.cacheDir, DIR)

    /** Saves [bytes] and returns the URI to put on the clipboard. */
    fun save(mime: String, bytes: ByteArray): Uri {
        dir.mkdirs()
        val file = File(dir, "clip-${System.currentTimeMillis()}.${extension(mime)}")
        file.writeBytes(bytes)
        prune()
        return ClipFileProvider.uriFor(context, file)
    }

    /** Deletes all but the newest [KEEP] images. */
    private fun prune() {
        val files = dir.listFiles()?.sortedByDescending { it.lastModified() } ?: return
        files.drop(KEEP).forEach { it.delete() }
    }

    companion object {
        /** The image types the engine accepts, as in `beam-core/src/clipboard.rs`. */
        val TYPES = setOf(
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/avif",
            "image/heic",
            "image/heif",
            "image/bmp"
        )

        internal const val DIR = "beam-clips"
        private const val KEEP = 3

        fun extension(mime: String): String = when (mime) {
            "image/jpeg" -> "jpg"
            else -> mime.substringAfter('/').ifEmpty { "img" }
        }
    }
}

/** Reads at most [limit] bytes; a result of [limit] bytes means there may be more. */
internal fun readAtMost(input: InputStream, limit: Int): ByteArray {
    val out = ByteArrayOutputStream()
    val buffer = ByteArray(64 * 1024)
    while (out.size() < limit) {
        val read = input.read(buffer, 0, minOf(buffer.size, limit - out.size()))
        if (read < 0) break
        out.write(buffer, 0, read)
    }
    return out.toByteArray()
}
