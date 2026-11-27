package app.beam.core.clipboard

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build

/**
 * What another app shared with "Share → Beam": text (a link, a message, a code) or one image.
 *
 * The intent comes from any app, so only `content:` images from other apps' providers are
 * read: a `file:` URI or one of Beam's own providers could otherwise make Beam send its own
 * private files.
 */
internal object SharedClip {
    fun read(context: Context, intent: Intent): ClipboardRead {
        if (intent.action != Intent.ACTION_SEND) return ClipboardRead.Unsupported
        val type = intent.type ?: return ClipboardRead.Unsupported
        return when {
            type.startsWith("image/") -> {
                val uri = stream(intent) ?: return ClipboardRead.Empty
                if (!isOthersContent(context, uri)) return ClipboardRead.Unsupported
                ClipReading.image(context, uri, sensitive = false)
            }
            type.startsWith("text/") ->
                ClipReading.text(intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString(), false)
            else -> ClipboardRead.Unsupported
        }
    }

    private fun stream(intent: Intent): Uri? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION") // the typed overload is Android 13+
            intent.getParcelableExtra(Intent.EXTRA_STREAM)
        }

    /** A `content:` URI served by another app, never by Beam itself. */
    private fun isOthersContent(context: Context, uri: Uri): Boolean {
        if (uri.scheme != ContentResolver.SCHEME_CONTENT) return false
        val authority = uri.authority ?: return false
        val manager = context.packageManager
        val provider = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            manager.resolveContentProvider(authority, PackageManager.ComponentInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION") // the typed flags are Android 13+
            manager.resolveContentProvider(authority, 0)
        }
        return provider != null && provider.packageName != context.packageName
    }
}
