package com.typixnode.watch.platform

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.typix_core.CoreException
import uniffi.typix_core.SecureStore

/**
 * The core's key storage.  An AES key that cannot leave the Android Keystore wraps each
 * value; only the wrapped form reaches app storage, which is excluded from backups.
 */
class KeystoreStore(context: Context) : SecureStore {
    private val prefs = context.getSharedPreferences("typix-keys", Context.MODE_PRIVATE)

    private fun masterKey(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as SecretKey?)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }

    override fun load(name: String): ByteArray? = guarded {
        val stored = prefs.getString(name, null) ?: return@guarded null
        val wrapped = Base64.decode(stored, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORM)
        val spec = GCMParameterSpec(128, wrapped, 0, IV_LEN)
        cipher.init(Cipher.DECRYPT_MODE, masterKey(), spec)
        cipher.updateAAD(name.toByteArray())
        cipher.doFinal(wrapped, IV_LEN, wrapped.size - IV_LEN)
    }

    override fun save(name: String, value: ByteArray) = guarded {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.ENCRYPT_MODE, masterKey())
        cipher.updateAAD(name.toByteArray())
        val wrapped = cipher.iv + cipher.doFinal(value)
        check(prefs.edit().putString(name, Base64.encodeToString(wrapped, Base64.NO_WRAP)).commit())
    }

    override fun remove(name: String) = guarded {
        check(prefs.edit().remove(name).commit())
    }

    private inline fun <T> guarded(body: () -> T): T = try {
        body()
    } catch (e: Exception) {
        throw CoreException.Storage(e.toString())
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "typix-watch-wrap"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
    }
}
