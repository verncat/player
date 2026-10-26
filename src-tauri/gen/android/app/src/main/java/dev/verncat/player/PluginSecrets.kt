package dev.verncat.player

import android.app.Activity
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
internal class PluginSecretArgs {
    lateinit var key: String
    var value: String = ""
}

/** Private encrypted values; encryption key never leaves AndroidKeyStore. */
@TauriPlugin
class PluginSecrets(private val activity: Activity) : Plugin(activity) {
    private val alias = "player.plugin.secrets.v1"
    private val preferences get() = activity.getSharedPreferences("plugin-secrets", Activity.MODE_PRIVATE)
    @Synchronized private fun encryptionKey(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Command fun write(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(PluginSecretArgs::class.java)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, encryptionKey()) }
            val encoded = Base64.encodeToString(cipher.iv, Base64.NO_WRAP) + ":" + Base64.encodeToString(cipher.doFinal(args.value.toByteArray(Charsets.UTF_8)), Base64.NO_WRAP)
            if (!preferences.edit().putString(args.key, encoded).commit()) throw IllegalStateException("Credential write failed")
            invoke.resolve(JSObject())
        } catch (e: Exception) { invoke.reject("Could not store plugin credential") }
    }
    @Command fun read(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(PluginSecretArgs::class.java)
            val encoded = preferences.getString(args.key, null)
            val result = JSObject()
            if (encoded == null) result.put("value", org.json.JSONObject.NULL)
            else {
                val parts = encoded.split(":", limit = 2)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, encryptionKey(), GCMParameterSpec(128, Base64.decode(parts[0], Base64.NO_WRAP))) }
                result.put("value", String(cipher.doFinal(Base64.decode(parts[1], Base64.NO_WRAP)), Charsets.UTF_8))
            }
            invoke.resolve(result)
        } catch (e: Exception) { invoke.reject("Could not read plugin credential") }
    }
    @Command fun delete(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(PluginSecretArgs::class.java)
            if (!preferences.edit().remove(args.key).commit()) throw IllegalStateException("Credential deletion failed")
            invoke.resolve(JSObject())
        } catch (e: Exception) { invoke.reject("Could not delete plugin credential") }
    }
}
