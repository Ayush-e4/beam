package app.beam.core.identity

import android.content.Context
import app.beam.core.BeamException
import app.beam.core.DeviceIdentity
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.security.GeneralSecurityException

/** How [IdentityStore.loadOrCreate] got the identity. */
enum class IdentityOrigin {
    /** Read back from storage: the usual case. */
    LOADED,

    /** First launch: no identity was stored yet. */
    CREATED,

    /**
     * The stored identity could not be read (its wrapping key is gone or the file is damaged),
     * so a new one replaced it. Trusted devices no longer recognise this phone and must pair
     * again; the app should say so.
     */
    REPLACED
}

/** The device identity plus how it was obtained, and why the old one was lost if it was. */
class StoredIdentity(
    val identity: DeviceIdentity,
    val origin: IdentityOrigin,
    val problem: Exception? = null
)

/**
 * Keeps the device's long-term identity in [file], encrypted by [wrapper]. The file belongs in
 * no-backup storage: the Keystore key that wraps it cannot move to another phone anyway.
 */
class IdentityStore(private val file: File, private val wrapper: KeyWrapper) {
    /** Returns the stored identity, creating and storing one if there is none or it's unreadable. */
    @Synchronized
    @Throws(IOException::class, GeneralSecurityException::class, BeamException::class)
    fun loadOrCreate(): StoredIdentity {
        if (!file.exists()) return StoredIdentity(createAndSave(), IdentityOrigin.CREATED)
        val problem =
            try {
                return StoredIdentity(load(), IdentityOrigin.LOADED)
            } catch (e: IOException) {
                e
            } catch (e: GeneralSecurityException) {
                e
            } catch (e: BeamException) {
                e
            }
        // Keep the unreadable file for diagnosis instead of destroying it.
        file.copyTo(File(file.path + UNREADABLE_SUFFIX), overwrite = true)
        return StoredIdentity(createAndSave(), IdentityOrigin.REPLACED, problem)
    }

    private fun load(): DeviceIdentity {
        val stored = file.readBytes()
        if (stored.isEmpty() || stored[0] != FORMAT_VERSION) {
            throw IOException("unknown identity file format")
        }
        val privateKey = wrapper.unwrap(stored.copyOfRange(1, stored.size))
        try {
            return DeviceIdentity.fromPrivateKey(privateKey)
        } finally {
            privateKey.fill(0)
        }
    }

    private fun createAndSave(): DeviceIdentity {
        val identity = DeviceIdentity.generate()
        val privateKey = identity.privateKey()
        val wrapped =
            try {
                wrapper.wrap(privateKey)
            } finally {
                privateKey.fill(0)
            }
        writeAtomically(byteArrayOf(FORMAT_VERSION) + wrapped)
        return identity
    }

    /** Writes a temporary file, syncs it and renames it over [file], so a crash never leaves half a key. */
    private fun writeAtomically(bytes: ByteArray) {
        file.parentFile?.mkdirs()
        val temporary = File(file.path + TEMPORARY_SUFFIX)
        FileOutputStream(temporary).use { out ->
            out.write(bytes)
            out.fd.sync()
        }
        if (!temporary.renameTo(file)) {
            temporary.delete()
            throw IOException("could not move the new identity into place at $file")
        }
    }

    companion object {
        private const val FORMAT_VERSION: Byte = 1
        private const val TEMPORARY_SUFFIX = ".tmp"
        private const val UNREADABLE_SUFFIX = ".unreadable"

        /** The app's identity store: wrapped by Android Keystore, kept out of backups. */
        fun forApp(context: Context): IdentityStore = IdentityStore(
            File(context.noBackupFilesDir, "device-identity.bin"),
            KeystoreKeyWrapper()
        )
    }
}
