package dev.verncat.player.example.nativeplugin

import android.app.Activity
import android.os.Bundle
import org.json.JSONObject

/** Versioned reflection ABI; all argument/result data crosses as JSON. */
class NativePlugin {
    init { System.loadLibrary("player_native_example") }

    fun apiVersion(): Int = 1
    fun pluginId(): String = "example.android-native"

    fun call(method: String, argsJson: String): String {
        val response = JSONObject(nativeCall(method, argsJson))
        check(response.getBoolean("ok")) { response.optString("error", "Native call failed") }
        return response.get("value").toString()
    }

    // Real plugins cancel workers, close sockets and release their instance state here.
    fun close() {}
    private external fun nativeCall(method: String, argsJson: String): String
}

class DiscoveryActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) { super.onCreate(savedInstanceState); finish() }
}
