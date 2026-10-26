package dev.verncat.player

import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ApkPluginsTest {
    @Test fun installedApkLoadsRustAndEnforcesIdentityAndLifecycle() {
        val runtime = InstalledApkRuntime(InstrumentationRegistry.getInstrumentation().targetContext)
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
            this.binding = binding; method = "echo"; argsJson = "{\"message\":\"Привет 🎵\"}"
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

        val first = runtime.call(args).getJSONObject("result")
        assertEquals("Rust via JNI", first.getString("backend"))
        assertEquals("Привет 🎵", first.getString("message"))
        assertEquals("aarch64", first.getString("arch"))
        runtime.deactivate(ApkDeactivateArgs().apply { pluginId = args.pluginId; generation = 2 })
        assertThrows(IllegalArgumentException::class.java) { runtime.call(args) }
        args.generation = 3
        val next = runtime.call(args).getJSONObject("result")
        assertTrue(next.getLong("callCount") > first.getLong("callCount"))
        runtime.deactivate(ApkDeactivateArgs().apply { pluginId = args.pluginId; generation = 4 })
    }
}
