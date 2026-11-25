package app.beam.core.trust

import android.content.Context
import android.util.Log
import app.beam.core.PairedDevice
import app.beam.core.SessionCipher
import app.beam.core.TrustedKeys
import app.beam.core.storage.writeAtomically
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.io.File
import java.io.IOException

/**
 * A device this phone paired with. [discoveryKey] is the only secret: it lets this phone
 * recognise the device's discovery beacons on the network.
 */
data class TrustedDevice(
    val deviceId: String,
    val publicKey: ByteArray,
    val name: String,
    val cipher: SessionCipher,
    val discoveryKey: ByteArray,
    val pairedAtMillis: Long
) {
    override fun equals(other: Any?): Boolean = other is TrustedDevice &&
        deviceId == other.deviceId &&
        publicKey.contentEquals(other.publicKey) &&
        name == other.name &&
        cipher == other.cipher &&
        discoveryKey.contentEquals(other.discoveryKey) &&
        pairedAtMillis == other.pairedAtMillis

    override fun hashCode(): Int = deviceId.hashCode()

    override fun toString(): String = "TrustedDevice(deviceId=$deviceId, name=$name)"

    companion object {
        /** The record to keep after pairing with [device] at [nowMillis]. */
        fun from(device: PairedDevice, nowMillis: Long) = TrustedDevice(
            device.deviceId,
            device.publicKey,
            device.name,
            device.cipher,
            device.discoveryKey,
            nowMillis
        )
    }
}

/**
 * The devices this phone trusts, kept in [file]. Reads come from memory; every change is
 * written atomically, so a crash never leaves a half-written list. Unreadable files throw
 * rather than silently forgetting every trusted device. Pass it to `Engine.listen` so each
 * offer says whether it comes from a trusted device.
 */
class TrustedDevices(
    private val file: File,
    /** Told when the engine asked and the list couldn't be read. */
    private val onUnreadable: (IOException) -> Unit = { e ->
        Log.e(TAG, "trusted devices unreadable; treating the sender as unknown", e)
    }
) : TrustedKeys {
    private var devices: List<TrustedDevice>? = null

    /** Every trusted device, most recently paired first. */
    @Synchronized
    @Throws(IOException::class)
    fun all(): List<TrustedDevice> = loaded().sortedByDescending { it.pairedAtMillis }

    /** The trusted device with [publicKey], if any: the check before accepting a transfer. */
    @Synchronized
    @Throws(IOException::class)
    fun find(publicKey: ByteArray): TrustedDevice? =
        loaded().firstOrNull { it.publicKey.contentEquals(publicKey) }

    /**
     * For the engine: whether [publicKey] belongs to a trusted device. If the list can't be
     * read, the sender is treated as unknown, so the person is asked rather than trusting it.
     */
    override fun isTrusted(publicKey: ByteArray): Boolean = try {
        find(publicKey) != null
    } catch (e: IOException) {
        onUnreadable(e)
        false
    }

    /** Adds [device], replacing an earlier record of the same device (re-pairing). */
    @Synchronized
    @Throws(IOException::class)
    fun add(device: TrustedDevice) {
        save(loaded().filterNot { it.deviceId == device.deviceId } + device)
    }

    /** Stops trusting the device with [deviceId]; returns whether it was trusted. */
    @Synchronized
    @Throws(IOException::class)
    fun remove(deviceId: String): Boolean {
        val before = loaded()
        val after = before.filterNot { it.deviceId == deviceId }
        if (after.size != before.size) save(after)
        return after.size != before.size
    }

    private fun loaded(): List<TrustedDevice> = devices ?: read().also { devices = it }

    private fun read(): List<TrustedDevice> {
        if (!file.exists()) return emptyList()
        try {
            DataInputStream(ByteArrayInputStream(file.readBytes())).use { input ->
                if (input.readByte() !=
                    FORMAT_VERSION
                ) {
                    throw IOException("unknown trusted-devices format")
                }
                return List(input.readInt()) { input.readDevice() }
            }
        } catch (e: EOFException) {
            throw IOException("trusted-devices file is truncated", e)
        }
    }

    private fun save(updated: List<TrustedDevice>) {
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).use { out ->
            out.writeByte(FORMAT_VERSION.toInt())
            out.writeInt(updated.size)
            updated.forEach { out.writeDevice(it) }
        }
        writeAtomically(file, bytes.toByteArray())
        devices = updated
    }

    private fun DataOutputStream.writeDevice(device: TrustedDevice) {
        writeUTF(device.deviceId)
        writeShort(device.publicKey.size)
        write(device.publicKey)
        writeUTF(device.name)
        writeUTF(device.cipher.name)
        writeShort(device.discoveryKey.size)
        write(device.discoveryKey)
        writeLong(device.pairedAtMillis)
    }

    private fun DataInputStream.readDevice(): TrustedDevice {
        val deviceId = readUTF()
        val key = ByteArray(readUnsignedShort()).also { readFully(it) }
        val name = readUTF()
        val cipherName = readUTF()
        val cipher =
            SessionCipher.entries.firstOrNull { it.name == cipherName }
                ?: throw IOException("unknown cipher \"$cipherName\" in trusted-devices file")
        val discoveryKey = ByteArray(readUnsignedShort()).also { readFully(it) }
        return TrustedDevice(deviceId, key, name, cipher, discoveryKey, readLong())
    }

    companion object {
        // 2 added the discovery key. Nothing shipped with 1, so it isn't migrated.
        private const val FORMAT_VERSION: Byte = 2
        private const val TAG = "BeamTrust"

        /** The app's trusted devices. Not backed up: the identity they trust isn't either. */
        fun forApp(context: Context) =
            TrustedDevices(File(context.noBackupFilesDir, "trusted-devices.bin"))
    }
}
