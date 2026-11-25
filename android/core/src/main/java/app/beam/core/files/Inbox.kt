package app.beam.core.files

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Environment
import android.os.storage.StorageManager
import app.beam.core.IncomingOffer
import app.beam.core.ProgressListener
import java.io.File
import java.io.IOException
import java.util.concurrent.atomic.AtomicInteger
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/** Whether received files can be saved to [Inbox.folder] right now. */
enum class InboxStatus {
    READY,

    /** Android 10 and older: the app must ask for permission to write to Download first. */
    NEEDS_PERMISSION,

    /** Shared storage isn't available (removed or being checked). */
    UNAVAILABLE
}

/**
 * Received files go to Download/Beam, where the Files app and the gallery show them. Files
 * are saved there directly; unfinished ones wait in a hidden folder inside it, so finishing
 * is a rename and a resumed transfer picks up where it stopped.
 */
object Inbox {
    /** Download/Beam on the phone's shared storage. */
    fun folder(context: Context): File = File(downloads(context), FOLDER)

    fun status(context: Context): InboxStatus = when {
        Environment.getExternalStorageState() != Environment.MEDIA_MOUNTED ->
            InboxStatus.UNAVAILABLE
        Build.VERSION.SDK_INT <= Build.VERSION_CODES.Q &&
            context.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) !=
            PackageManager.PERMISSION_GRANTED -> InboxStatus.NEEDS_PERMISSION
        else -> InboxStatus.READY
    }

    /**
     * The folder, created if needed.
     *
     * @throws IOException if it can't be used now; [status] says why.
     */
    fun prepare(context: Context): File {
        val status = status(context)
        if (status != InboxStatus.READY) throw IOException("can't save to Download/Beam: $status")
        val folder = folder(context)
        if (!folder.isDirectory && !folder.mkdirs()) throw IOException("can't create $folder")
        return folder
    }

    /**
     * Tells the media index about [paths], so the gallery and other apps see them. Returns
     * once Android has indexed them, or after a few seconds; indexing finishes regardless.
     */
    suspend fun announce(context: Context, paths: List<String>) {
        if (paths.isEmpty()) return
        withTimeoutOrNull(ANNOUNCE_WAIT_MS) {
            suspendCancellableCoroutine { done ->
                val left = AtomicInteger(paths.size)
                MediaScannerConnection.scanFile(context, paths.toTypedArray(), null) { _, _ ->
                    if (left.decrementAndGet() == 0 && done.isActive) done.resume(Unit)
                }
            }
        }
    }

    private fun downloads(context: Context): File =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val storage = context.getSystemService(StorageManager::class.java)
            val root = storage.primaryStorageVolume.directory
                ?: throw IOException("shared storage isn't available")
            File(root, Environment.DIRECTORY_DOWNLOADS)
        } else {
            // The only way to find it before Android 11.
            @Suppress("DEPRECATION")
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        }

    private const val FOLDER = "Beam"
    private const val ANNOUNCE_WAIT_MS = 10_000L
}

/**
 * Receives the offer into Download/Beam and makes the files visible to other apps. Returns
 * the saved paths. [selection] lists item IDs to take, empty for all.
 *
 * @throws IOException if Download/Beam can't be used ([Inbox.status] says why).
 * @throws app.beam.core.BeamException as [IncomingOffer.accept].
 */
suspend fun IncomingOffer.acceptIntoInbox(
    context: Context,
    progress: ProgressListener,
    selection: List<UInt> = emptyList()
): List<String> {
    val folder = Inbox.prepare(context)
    val saved = accept(folder.path, selection, progress)
    Inbox.announce(context, saved)
    return saved
}
