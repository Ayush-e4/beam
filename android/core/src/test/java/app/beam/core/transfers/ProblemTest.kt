package app.beam.core.transfers

import app.beam.core.NetworkFailure.NO_ANSWER
import app.beam.core.NetworkFailure.OTHER
import app.beam.core.NetworkFailure.REFUSED
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ProblemTest {
    private val local = true
    private val remote = false

    @Test
    fun `a refusal anywhere means Beam isn't open there`() {
        val attempts = listOf(Attempt(NO_ANSWER, local), Attempt(REFUSED, local))
        assertEquals(Problem.NOT_RECEIVING, problemOf(attempts, behindVpn = true))
    }

    @Test
    fun `no answer behind a VPN is the VPN`() {
        val attempts = listOf(Attempt(NO_ANSWER, local))
        assertEquals(Problem.VPN_BLOCKS, problemOf(attempts, behindVpn = true))
    }

    @Test
    fun `no answer on this phone's own network is the network`() {
        val attempts = listOf(Attempt(NO_ANSWER, remote), Attempt(NO_ANSWER, local))
        assertEquals(Problem.NETWORK_BLOCKS_DEVICES, problemOf(attempts, behindVpn = false))
    }

    @Test
    fun `no answer elsewhere, or no address at all, is out of reach`() {
        val attempts = listOf(Attempt(NO_ANSWER, remote))
        assertEquals(Problem.OUT_OF_REACH, problemOf(attempts, behindVpn = false))
        assertEquals(Problem.OUT_OF_REACH, problemOf(emptyList(), behindVpn = false))
    }

    @Test
    fun `a link that dropped partway is no problem to explain`() {
        val attempts = listOf(Attempt(NO_ANSWER, local), Attempt(OTHER, local))
        assertNull(problemOf(attempts, behindVpn = true))
    }
}
