package app.beam.core.files

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.util.Log
import app.beam.core.Engine
import app.beam.core.OutgoingTransfer
import app.beam.core.SharedFile
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Prepares a transfer of content [uris] from the gallery, the share sheet or a file picker.
 * Each file is sent under its display name, straight from the provider, without copying.
 * Sources that can only be streamed are first copied into the app's cache.
 *
 * @throws IOException if one can't be opened, or Beam may no longer read it.
 * @throws app.beam.core.BeamException if the selection can't be sent.
 */
suspend fun Engine.prepareSendUris(context: Context, uris: List<Uri>): OutgoingTransfer =
    withContext(Dispatchers.IO) {
        val resolver = context.contentResolver
        val spillDir = File(context.cacheDir, SPILL_DIR)
        val open = ArrayList<ParcelFileDescriptor>(uris.size)
        try {
            val files = uris.map { uri ->
                val name = displayName(resolver, uri)
                val source = openForReading(resolver, uri, name).also(open::add)
                val readable = readableAtAnyPosition(source, spillDir).also {
                    if (it !== source) open += it
                }
                SharedFile(name, readable.fd)
            }
            // The engine takes its own descriptors before this returns.
            prepareSendShared(files)
        } finally {
            open.forEach(::closeOrLog)
        }
    }

/** The name the provider shows for [uri], or its last path segment. */
internal fun displayName(resolver: ContentResolver, uri: Uri): String {
    val shown = try {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst() && !it.isNull(0)) it.getString(0) else null
        }
    } catch (e: SecurityException) {
        Log.w(TAG, "no name for $uri", e)
        null
    }
    return shown?.takeIf { it.isNotBlank() } ?: uri.lastPathSegment ?: FALLBACK_NAME
}

private fun openForReading(resolver: ContentResolver, uri: Uri, name: String) = try {
    resolver.openFileDescriptor(uri, "r") ?: throw FileNotFoundException("$name is gone")
} catch (e: SecurityException) {
    throw IOException("Beam may no longer read $name", e)
}

/**
 * [source] itself if it is a regular file, which the engine reads at any position. Otherwise
 * (a pipe or socket from a streaming provider) a copy in [spillDir], already deleted from it:
 * the open descriptor keeps it readable, and the space is freed once the transfer lets go.
 */
internal fun readableAtAnyPosition(
    source: ParcelFileDescriptor,
    spillDir: File
): ParcelFileDescriptor {
    val mode = try {
        Os.fstat(source.fileDescriptor).st_mode
    } catch (e: ErrnoException) {
        throw IOException("could not inspect a shared file", e)
    }
    if (OsConstants.S_ISREG(mode)) return source

    spillDir.mkdirs()
    val copy = File.createTempFile("shared", null, spillDir)
    try {
        ParcelFileDescriptor.AutoCloseInputStream(source.dup()).use { input ->
            copy.outputStream().use { input.copyTo(it) }
        }
        return ParcelFileDescriptor.open(copy, ParcelFileDescriptor.MODE_READ_ONLY)
    } finally {
        if (!copy.delete()) Log.w(TAG, "could not delete $copy")
    }
}

private fun closeOrLog(descriptor: ParcelFileDescriptor) {
    try {
        descriptor.close()
    } catch (e: IOException) {
        // Read-only: nothing is lost, but a provider that fails to close is worth a log.
        Log.w(TAG, "could not close a shared file", e)
    }
}

private const val TAG = "BeamFiles"
private const val SPILL_DIR = "beam-outgoing"
private const val FALLBACK_NAME = "file"
