package com.reflow.android.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LanTest {
    @Test fun acceptsLiteralPrivateAddressesAndLoopback() {
        listOf("127.0.0.1", "localhost", "::1", "10.1.2.3", "192.168.0.2", "172.31.0.1")
            .forEach { assertTrue(it, isAllowedLanHost(it)) }
    }

    @Test fun rejectsPublicMalformedAndDnsHostnames() {
        listOf("8.8.8.8", "172.32.0.1", "192.168.1.999", "localhost.localdomain", "127.1", "2130706433")
            .forEach { assertFalse(it, isAllowedLanHost(it)) }
    }

    @Test fun pairingUriRejectsPortsOutsideTheNetworkRange() {
        assertNull(parsePairUri("reflow://pair?v=2&host=192.168.1.2&port=0&code=123456&cert_sha256=${"a".repeat(64)}"))
        assertNull(parsePairUri("reflow://pair?v=2&host=192.168.1.2&port=99999&code=123456&cert_sha256=${"a".repeat(64)}"))
    }

    @Test fun pairingUriValidatesAndDecodesLoopbackIpv6() {
        assertEquals(PairOffer("::1", 7840, "123456", "a".repeat(64)), parsePairUri("reflow://pair?v=2&host=%3A%3A1&port=7840&code=123456&cert_sha256=${"a".repeat(64)}"))
    }
}
