package app.beam.core.network

/** How a Wi-Fi network checks its password, named as Wi-Fi QR codes name it. */
enum class WifiSecurity(internal val qrName: String) {
    /** No password. */
    OPEN("nopass"),

    /** WPA2, or WPA3 that still lets WPA2 devices join. */
    WPA("WPA"),

    /** WPA3 only. */
    SAE("SAE")
}

/**
 * The standard Wi-Fi QR code text (`WIFI:T:WPA;S:name;P:password;;`). Phone and tablet cameras
 * offer to join the network when they see it.
 */
fun wifiQr(ssid: String, passphrase: String?, security: WifiSecurity): String = buildString {
    append("WIFI:T:").append(security.qrName)
    append(";S:").append(escaped(ssid))
    if (security != WifiSecurity.OPEN && passphrase != null) {
        append(";P:").append(escaped(passphrase))
    }
    append(";;")
}

/** The QR format escapes its own separators with a backslash. */
private fun escaped(text: String) = text.replace(SPECIAL) { "\\" + it.value }

private val SPECIAL = Regex("""[\\;,:"]""")
