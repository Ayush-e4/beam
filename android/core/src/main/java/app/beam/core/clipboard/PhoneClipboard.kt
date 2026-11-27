package app.beam.core.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import app.beam.core.ClipBody
import app.beam.core.ReceivedClip

/**
 * The phone's clipboard: puts what a linked device copied on it, and reads what to send.
 *
 * Android 10+ only lets an app read the clipboard while one of its windows has focus
 * ([ClipFocusActivity] gives Beam one for a moment); writing works any time.
 */
class PhoneClipboard(private val context: Context) {
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val images = ClipImages(context)

    /** Puts [clip] on the clipboard, marked sensitive (Android 13+) when it is. */
    fun put(clip: ReceivedClip) {
        val data = when (val body = clip.body) {
            is ClipBody.Text -> ClipData.newPlainText(LABEL, body.text)
            is ClipBody.Image -> ClipData.newUri(
                context.contentResolver,
                LABEL,
                images.save(body.mime, body.bytes)
            )
        }
        if (clip.sensitive) markSensitive(data.description)
        manager.setPrimaryClip(data)
    }

    /** Reads the clipboard. Only finds something while a Beam window has focus. */
    fun read(): ClipboardRead {
        val data = manager.primaryClip ?: return ClipboardRead.Empty
        if (data.itemCount == 0) return ClipboardRead.Empty
        val item = data.getItemAt(0)
        val sensitive = isSensitive(data.description)
        val uri = item.uri
        if (uri != null && item.text == null) return ClipReading.image(context, uri, sensitive)
        return ClipReading.text(item.coerceToText(context)?.toString(), sensitive)
    }

    private fun markSensitive(description: ClipDescription) {
        description.extras = PersistableBundle().apply { putBoolean(EXTRA_IS_SENSITIVE, true) }
    }

    private fun isSensitive(description: ClipDescription): Boolean =
        description.extras?.getBoolean(EXTRA_IS_SENSITIVE, false) == true

    companion object {
        private const val LABEL = "Beam"

        /**
         * Android 13's `ClipDescription.EXTRA_IS_SENSITIVE`; older versions ignore it, and
         * keyboards and clipboard managers that know it already honour it there too.
         */
        private val EXTRA_IS_SENSITIVE = if (Build.VERSION.SDK_INT >=
            Build.VERSION_CODES.TIRAMISU
        ) {
            ClipDescription.EXTRA_IS_SENSITIVE
        } else {
            "android.content.extra.IS_SENSITIVE"
        }
    }
}
