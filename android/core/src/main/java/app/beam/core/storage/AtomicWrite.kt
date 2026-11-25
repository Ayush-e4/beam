package app.beam.core.storage

import java.io.File
import java.io.FileOutputStream
import java.io.IOException

/**
 * Replaces [file] with [bytes] so a crash leaves either the old or the new contents, never
 * half of them: written to a temporary file, synced, then renamed over it.
 *
 * @throws IOException if it can't be written.
 */
internal fun writeAtomically(file: File, bytes: ByteArray) {
    file.parentFile?.mkdirs()
    val temporary = File(file.path + ".tmp")
    FileOutputStream(temporary).use { out ->
        out.write(bytes)
        out.fd.sync()
    }
    if (!temporary.renameTo(file)) {
        temporary.delete()
        throw IOException("could not save $file")
    }
}
