package app.beam.core.trust

import app.beam.core.SessionCipher
import java.io.File
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class TrustedDevicesTest {
    @get:Rule val folder = TemporaryFolder()

    private val file get() = File(folder.root, "trust/trusted-devices.bin")

    private fun device(n: Int, name: String = "Phone $n", at: Long = n.toLong()) =
        TrustedDevice("id-$n", ByteArray(32) { n.toByte() }, name, SessionCipher.AES_GCM, at)

    @Test
    fun `devices survive a restart, newest first`() {
        TrustedDevices(file).apply {
            add(device(1))
            add(device(2))
        }
        assertEquals(listOf(device(2), device(1)), TrustedDevices(file).all())
    }

    @Test
    fun `every cipher survives a restart`() {
        val devices = SessionCipher.entries.mapIndexed { n, cipher ->
            device(n).copy(cipher = cipher)
        }
        TrustedDevices(file).apply { devices.forEach(::add) }
        assertEquals(devices.reversed(), TrustedDevices(file).all())
    }

    @Test
    fun `re-pairing replaces the old record`() {
        val store = TrustedDevices(file)
        store.add(device(1))
        store.add(device(1, name = "Renamed", at = 10))
        assertEquals(listOf(device(1, name = "Renamed", at = 10)), TrustedDevices(file).all())
    }

    @Test
    fun `devices are found by public key`() {
        val store = TrustedDevices(file)
        store.add(device(1))
        store.add(device(2))
        assertEquals("Phone 2", store.find(ByteArray(32) { 2 })?.name)
        assertNull(store.find(ByteArray(32) { 9 }))
    }

    @Test
    fun `removing stops trusting a device`() {
        val store = TrustedDevices(file)
        store.add(device(1))
        assertTrue(store.remove("id-1"))
        assertFalse(store.remove("id-1"))
        assertEquals(emptyList<TrustedDevice>(), TrustedDevices(file).all())
        assertFalse(File(file.path + ".tmp").exists())
    }

    @Test
    fun `no file means no trusted devices`() {
        assertEquals(emptyList<TrustedDevice>(), TrustedDevices(file).all())
    }

    @Test
    fun `a damaged file is an error, not an empty list`() {
        TrustedDevices(file).add(device(1))
        file.writeBytes(file.readBytes().copyOf(10))
        assertThrows(IOException::class.java) { TrustedDevices(file).all() }
        file.writeBytes(byteArrayOf(7))
        assertThrows(IOException::class.java) { TrustedDevices(file).all() }
    }
}
