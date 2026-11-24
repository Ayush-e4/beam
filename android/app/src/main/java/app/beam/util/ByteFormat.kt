package app.beam.util

import java.util.Locale

private const val UNIT = 1024.0
private val UNITS = listOf("B", "KB", "MB", "GB", "TB")

/**
 * Formats a byte count for display, e.g. `1536` -> `1.5 KB`.
 *
 * @throws IllegalArgumentException if [bytes] is negative.
 */
fun formatBytes(bytes: Long): String {
    require(bytes >= 0) { "Byte count must not be negative: $bytes" }
    if (bytes < UNIT) return "$bytes B"

    var value = bytes.toDouble()
    var unitIndex = 0
    while (value >= UNIT && unitIndex < UNITS.lastIndex) {
        value /= UNIT
        unitIndex++
    }
    return String.format(Locale.ROOT, "%.1f %s", value, UNITS[unitIndex])
}
