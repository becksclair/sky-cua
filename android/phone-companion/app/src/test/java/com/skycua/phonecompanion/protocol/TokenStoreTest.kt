package com.skycua.phonecompanion.protocol

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TokenStoreTest {
    @Test
    fun validTokenBeforeExpiry() {
        val store = TokenStore().apply { install("tok", expiresAtMs = 1000) }
        assertTrue(store.isValid("tok", nowMs = 999))
    }

    @Test
    fun rejectsAtAndAfterExpiry() {
        val store = TokenStore().apply { install("tok", expiresAtMs = 1000) }
        assertFalse(store.isValid("tok", nowMs = 1000))
        assertFalse(store.isValid("tok", nowMs = 1001))
    }

    @Test
    fun rejectsWrongToken() {
        val store = TokenStore().apply { install("tok", expiresAtMs = 1000) }
        assertFalse(store.isValid("nope", nowMs = 500))
    }

    @Test
    fun rejectsNullToken() {
        val store = TokenStore().apply { install("tok", expiresAtMs = 1000) }
        assertFalse(store.isValid(null, nowMs = 500))
    }

    @Test
    fun rejectsWhenNoTokenInstalled() {
        assertFalse(TokenStore().isValid("anything", nowMs = 0))
    }

    @Test
    fun clearRevokesToken() {
        val store = TokenStore().apply { install("tok", expiresAtMs = 1000) }
        store.clear()
        assertFalse(store.isValid("tok", nowMs = 0))
        assertFalse(store.hasToken)
    }

    @Test
    fun constantTimeEqualsHandlesLengthAndContent() {
        assertTrue(TokenStore.constantTimeEquals("abc", "abc"))
        assertFalse(TokenStore.constantTimeEquals("abc", "abcd"))
        assertFalse(TokenStore.constantTimeEquals("abc", "abd"))
    }
}
