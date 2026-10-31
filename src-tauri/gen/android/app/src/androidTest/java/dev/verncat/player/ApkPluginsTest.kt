package dev.verncat.player

import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ApkPluginsTest {
    @Test fun installedApkLoadsRustAndEnforcesIdentityAndLifecycle() {
        var settings = JSONObject("{\"revision\":0,\"values\":{}}")
        val runtime = InstalledApkRuntime(InstrumentationRegistry.getInstrumentation().targetContext) { _, method, arguments ->
            val args = JSONObject(arguments)
            val result = when (method) {
                "settings.get" -> settings
                "settings.update" -> JSONObject().put("revision", settings.getInt("revision") + 1).put("values", args.getJSONObject("values")).also { settings = it }
                "host.invalidate" -> JSONObject.NULL
                else -> error("Unexpected host callback: $method")
            }
            JSONObject().put("ok", true).put("value", result).toString()
        }
        val discovery = runtime.discover()
        val packages = discovery.getJSONArray("packages")
        val candidate = (0 until packages.length()).map { JSONObject(packages.getString(it)) }
            .firstOrNull { it.getJSONObject("manifest").getString("id") == "example.android-native" }
            ?: error("Install the android-native example APK before running this test: $discovery")
        val pinned = candidate.getJSONObject("manifest").getJSONObject("androidNative")
        val binding = ApkBinding().apply {
            packageName = pinned.getString("packageName")
            entryClass = pinned.getString("entryClass")
            certificateSha256 = pinned.getString("certificateSha256")
            apkSha256 = pinned.getString("apkSha256")
        }
        val args = ApkCallArgs().apply {
            pluginId = "example.android-native"; generation = 1
            this.binding = binding; method = "activate"; argsJson = "{}"
        }
        // Validate pinning before any third-party class is initialized.
        val hash = binding.apkSha256
        binding.apkSha256 = "0".repeat(64)
        assertThrows(IllegalArgumentException::class.java) { runtime.call(args) }
        binding.apkSha256 = hash
        val certificate = binding.certificateSha256
        binding.certificateSha256 = "0".repeat(64)
        assertThrows(IllegalArgumentException::class.java) { runtime.call(args) }
        binding.certificateSha256 = certificate

        assertFalse(candidate.has("script"))
        assertEquals("android-apk", candidate.getJSONObject("native").getString("kind"))
        runtime.call(args)
        args.method = "action"; args.argsJson = "{\"action\":\"ping\",\"fields\":{\"message\":\"Native Rust\"}}"
        runtime.call(args)
        args.method = "render"; args.argsJson = "{}"
        val first = runtime.call(args).getJSONObject("result")
        assertTrue(first.toString().contains("Rust via JNI"))
        assertTrue(first.toString().contains("Native Rust"))
        assertEquals("Native Rust", settings.getJSONObject("values").getString("lastMessage"))
        runtime.deactivate(ApkDeactivateArgs().apply { pluginId = args.pluginId; generation = 2 })
        assertThrows(IllegalArgumentException::class.java) { runtime.call(args) }
        args.generation = 3; args.method = "activate"
        runtime.call(args)
        args.method = "render"
        assertTrue(runtime.call(args).getJSONObject("result").toString().contains("Native Rust"))
        runtime.deactivate(ApkDeactivateArgs().apply { pluginId = args.pluginId; generation = 4 })
    }
}
