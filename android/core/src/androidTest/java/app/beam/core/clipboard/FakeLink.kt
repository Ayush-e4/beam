package app.beam.core.clipboard

import app.beam.core.ClipLinkInterface
import app.beam.core.ReceivedClip
import kotlinx.coroutines.channels.Channel

/**
 * A linked device played by the test: [arrive] is what it copies, [sent] what it got, as
 * `"text"`/`"secret"` to the text, or the image's type to its bytes.
 */
class FakeLink :
    ClipLinkInterface,
    AutoCloseable {
    val arrive = Channel<ReceivedClip>(Channel.UNLIMITED)
    val sent = Channel<Pair<String, Any>>(Channel.UNLIMITED)

    override suspend fun nextClip(): ReceivedClip? = arrive.receiveCatching().getOrNull()

    override fun peerKey(): ByteArray = ByteArray(32)

    override suspend fun sendText(text: String, sensitive: Boolean): Boolean {
        sent.send((if (sensitive) "secret" else "text") to text)
        return true
    }

    override suspend fun sendImage(mime: String, bytes: ByteArray, sensitive: Boolean): Boolean {
        sent.send(mime to bytes)
        return true
    }

    override fun stop() {
        arrive.close()
    }

    override fun close() = Unit
}
