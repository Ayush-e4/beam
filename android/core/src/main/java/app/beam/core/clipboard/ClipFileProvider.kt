package app.beam.core.clipboard

import android.content.Context
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.File

/**
 * Serves received clipboard images to the apps they are pasted into (paths in
 * `res/xml/beam_clip_paths.xml`). Its own class, so it never clashes with an app's provider.
 */
class ClipFileProvider : FileProvider() {
    companion object {
        /** The URI another app may read [file] through, once it is on the clipboard. */
        fun uriFor(context: Context, file: File): Uri =
            getUriForFile(context, "${context.packageName}.beam.clips", file)
    }
}
