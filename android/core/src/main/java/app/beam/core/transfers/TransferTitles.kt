package app.beam.core.transfers

import android.content.Context
import app.beam.core.R
import app.beam.core.TransferSummary

/**
 * The one name a transfer of [paths] goes by: the file's name, or the folder's when every file
 * is inside one folder. Null when there is no single name ("12 files").
 */
internal fun singleName(paths: List<String>): String? {
    val tops = paths.map { it.substringBefore('/') }.distinct()
    return tops.singleOrNull()?.takeIf { it.isNotEmpty() }
}

/** What a transfer is about, such as "photo.jpg", "Holiday" or "12 files". */
internal fun TransferSummary.title(context: Context): String = singleName(items.map { it.path })
    ?: context.resources.getQuantityString(R.plurals.beam_files, items.size, items.size)
