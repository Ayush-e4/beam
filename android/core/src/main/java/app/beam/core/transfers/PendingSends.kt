package app.beam.core.transfers

import app.beam.core.storage.writeAtomically
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.io.File
import java.io.IOException

/** How a send names its files. */
enum class SourceKind {
    /** File and folder paths the app may open. */
    PATHS,

    /** Content URIs from the gallery, the share sheet or a picker. */
    CONTENT
}

/** A send that hasn't been delivered yet, remembered so it resumes after the app restarts. */
data class PendingSend(
    /** As [app.beam.core.TransferSummary.transferId]; the receiver continues under it. */
    val transferId: String,
    /** The trusted device it goes to. */
    val deviceId: String,
    val title: String,
    val kind: SourceKind,
    /** Paths or URIs, by [kind]. */
    val sources: List<String>,
    val startedAtMillis: Long
)

/**
 * Sends to resume, kept in [file] until each is delivered, declined, cancelled by the person
 * or can no longer be sent. Every change is written atomically; an unreadable file throws
 * rather than silently forgetting the sends.
 */
class PendingSends(private val file: File) {
    private var sends: List<PendingSend>? = null

    /** Every remembered send, oldest first. */
    @Synchronized
    @Throws(IOException::class)
    fun all(): List<PendingSend> = loaded().sortedBy { it.startedAtMillis }

    /** Remembers [send], replacing an earlier record of the same transfer. */
    @Synchronized
    @Throws(IOException::class)
    fun add(send: PendingSend) {
        save(loaded().filterNot { it.transferId == send.transferId } + send)
    }

    /** Forgets [transferId]; returns whether it was remembered. */
    @Synchronized
    @Throws(IOException::class)
    fun remove(transferId: String): Boolean {
        val before = loaded()
        val after = before.filterNot { it.transferId == transferId }
        if (after.size != before.size) save(after)
        return after.size != before.size
    }

    private fun loaded(): List<PendingSend> = sends ?: read().also { sends = it }

    private fun read(): List<PendingSend> {
        if (!file.exists()) return emptyList()
        try {
            DataInputStream(ByteArrayInputStream(file.readBytes())).use { input ->
                if (input.readByte() != FORMAT_VERSION) {
                    throw IOException("unknown pending-sends format")
                }
                return List(input.readInt()) { input.readSend() }
            }
        } catch (e: EOFException) {
            throw IOException("pending-sends file is truncated", e)
        }
    }

    private fun save(updated: List<PendingSend>) {
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).use { out ->
            out.writeByte(FORMAT_VERSION.toInt())
            out.writeInt(updated.size)
            updated.forEach { out.writeSend(it) }
        }
        writeAtomically(file, bytes.toByteArray())
        sends = updated
    }

    private fun DataOutputStream.writeSend(send: PendingSend) {
        writeUTF(send.transferId)
        writeUTF(send.deviceId)
        writeUTF(send.title)
        writeUTF(send.kind.name)
        writeInt(send.sources.size)
        send.sources.forEach { writeUTF(it) }
        writeLong(send.startedAtMillis)
    }

    private fun DataInputStream.readSend(): PendingSend {
        val transferId = readUTF()
        val deviceId = readUTF()
        val title = readUTF()
        val kindName = readUTF()
        val kind = SourceKind.entries.firstOrNull { it.name == kindName }
            ?: throw IOException("unknown source kind \"$kindName\" in pending-sends file")
        val sources = List(readInt()) { readUTF() }
        return PendingSend(transferId, deviceId, title, kind, sources, readLong())
    }

    companion object {
        private const val FORMAT_VERSION: Byte = 1

        /** The app's pending sends. */
        fun forApp(context: android.content.Context) =
            PendingSends(File(context.noBackupFilesDir, "pending-sends.bin"))
    }
}
