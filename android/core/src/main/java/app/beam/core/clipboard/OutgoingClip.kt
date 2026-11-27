package app.beam.core.clipboard

/** Something on this phone's clipboard, ready to send to linked devices. */
sealed interface OutgoingClip {
    /** Copied text; [sensitive] for passwords and one-time codes. */
    data class Text(val text: String, val sensitive: Boolean) : OutgoingClip

    /** A copied image of a type the engine accepts. */
    class Image(val mime: String, val bytes: ByteArray, val sensitive: Boolean) : OutgoingClip
}

/** What reading the clipboard found. */
sealed interface ClipboardRead {
    /** Something to send. */
    data class Found(val clip: OutgoingClip) : ClipboardRead

    /** The clipboard is empty, or Android didn't let Beam read it (no window in focus). */
    data object Empty : ClipboardRead

    /** Too big for the clipboard link; send it as a file instead. */
    data object TooLarge : ClipboardRead

    /** Something Beam can't send as a clip, such as a video or an unreadable file. */
    data object Unsupported : ClipboardRead
}

/** How sending the clipboard to the linked devices went. */
enum class SendResult {
    /** At least one linked device got it. */
    SENT,

    /** Every linked device already has it (it came from one of them, or was sent before). */
    ALREADY_THERE,

    /** No device is linked. */
    NOBODY_LINKED,

    /** The clipboard is empty or couldn't be read. */
    NOTHING_COPIED,

    /** Too big for the clipboard link. */
    TOO_LARGE,

    /** Not something Beam can send as a clip. */
    UNSUPPORTED,

    /** The link failed while sending; the device is no longer linked. */
    FAILED
}
