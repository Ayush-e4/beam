package app.beam.core.history

import app.beam.core.transfers.Direction
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.io.IOException

/** The history file: a format byte, the entry count, then each entry. */
internal object HistoryFormat {
    private const val VERSION: Byte = 1

    fun encode(entries: List<HistoryEntry>): ByteArray {
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).use { out ->
            out.writeByte(VERSION.toInt())
            out.writeInt(entries.size)
            entries.forEach { out.writeEntry(it) }
        }
        return bytes.toByteArray()
    }

    /** @throws IOException if [bytes] aren't a history this version can read. */
    fun decode(bytes: ByteArray): List<HistoryEntry> {
        try {
            DataInputStream(ByteArrayInputStream(bytes)).use { input ->
                if (input.readByte() != VERSION) throw IOException("unknown history format")
                val entries = List(input.readInt()) { input.readEntry() }
                if (input.read() != -1) throw IOException("history file has trailing bytes")
                return entries
            }
        } catch (e: EOFException) {
            throw IOException("history file is truncated", e)
        }
    }

    private fun DataOutputStream.writeEntry(entry: HistoryEntry) {
        writeUTF(entry.transferId)
        writeUTF(entry.direction.name)
        writeUTF(entry.deviceId)
        writeUTF(entry.deviceName)
        writeUTF(entry.title)
        writeInt(entry.files.size)
        entry.files.forEach { file ->
            writeUTF(file.path)
            writeLong(file.size)
        }
        writeInt(entry.fileCount)
        writeLong(entry.totalBytes)
        writeUTF(entry.outcome.name)
        writeBoolean(entry.detail != null)
        entry.detail?.let { writeUTF(it) }
        writeLong(entry.atMillis)
        writeInt(entry.savedPaths.size)
        entry.savedPaths.forEach { writeUTF(it) }
    }

    private fun DataInputStream.readEntry(): HistoryEntry {
        val transferId = readUTF()
        val direction = named<Direction>(readUTF())
        val deviceId = readUTF()
        val deviceName = readUTF()
        val title = readUTF()
        val files = List(readCount()) { HistoryFile(readUTF(), readLong()) }
        val fileCount = readInt()
        val totalBytes = readLong()
        val outcome = named<Outcome>(readUTF())
        val detail = if (readBoolean()) readUTF() else null
        val atMillis = readLong()
        val savedPaths = List(readCount()) { readUTF() }
        return HistoryEntry(
            transferId, direction, deviceId, deviceName, title, files, fileCount, totalBytes,
            outcome, detail, atMillis, savedPaths
        )
    }

    /** A list length, refused when negative or beyond what the store ever writes. */
    private fun DataInputStream.readCount(): Int = readInt().also { count ->
        if (count !in 0..TransferHistory.KEPT_FILES) throw IOException("bad count in history")
    }

    private inline fun <reified E : Enum<E>> named(name: String): E =
        enumValues<E>().firstOrNull { it.name == name }
            ?: throw IOException("unknown ${E::class.simpleName} \"$name\" in history")
}
