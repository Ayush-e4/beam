package app.beam.core.clipboard

import android.content.Context
import android.net.Uri
import android.util.Log
import app.beam.core.ClipKind
import app.beam.core.clipKind
import java.io.IOException

/** Turns copied or shared text and images into clips, within the engine's limits. */
internal object ClipReading {
    /** The engine's limits for one clip. */
    const val MAX_TEXT_BYTES = 1024 * 1024
    const val MAX_IMAGE_BYTES = 8 * 1024 * 1024

    private const val TAG = "BeamClipboard"

    /** [text] as a clip; one-time codes always go as sensitive. */
    fun text(text: String?, sensitive: Boolean): ClipboardRead {
        if (text.isNullOrEmpty()) return ClipboardRead.Empty
        if (text.toByteArray().size > MAX_TEXT_BYTES) return ClipboardRead.TooLarge
        val code = clipKind(text) == ClipKind.CODE
        return ClipboardRead.Found(OutgoingClip.Text(text, sensitive || code))
    }

    /** The image at [uri] as a clip, if it is one of the types the engine accepts. */
    fun image(context: Context, uri: Uri, sensitive: Boolean): ClipboardRead {
        val resolver = context.contentResolver
        val mime = resolver.getType(uri)
        if (mime == null || mime !in ClipImages.TYPES) return ClipboardRead.Unsupported
        return try {
            val stream = resolver.openInputStream(uri) ?: return ClipboardRead.Unsupported
            val bytes = stream.use { readAtMost(it, MAX_IMAGE_BYTES + 1) }
            when {
                bytes.isEmpty() -> ClipboardRead.Empty
                bytes.size > MAX_IMAGE_BYTES -> ClipboardRead.TooLarge
                else -> ClipboardRead.Found(OutgoingClip.Image(mime, bytes, sensitive))
            }
        } catch (e: IOException) {
            Log.w(TAG, "could not read the image", e)
            ClipboardRead.Unsupported
        } catch (e: SecurityException) {
            Log.w(TAG, "not allowed to read the image", e)
            ClipboardRead.Unsupported
        }
    }
}
