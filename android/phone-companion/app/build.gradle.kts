import java.io.File
import java.security.MessageDigest
import java.security.cert.CertificateFactory
import java.util.zip.ZipFile
import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Optional
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction

plugins {
    alias(libs.plugins.android.application)
}

/**
 * Configuration-cache compatible task to emit companion build metadata.
 * All inputs are declared as task properties so the configuration cache can serialize the task graph.
 */
abstract class EmitBuildMetadataTask : DefaultTask() {
    @get:Input
    abstract val packageName: Property<String>

    @get:Input
    abstract val versionCode: Property<Int>

    @get:Input
    abstract val versionName: Property<String>

    @get:InputFile
    @get:Optional
    abstract val apkFile: RegularFileProperty

    @get:Input
    abstract val repoRootPath: Property<String>

    @get:Input
    @get:Optional
    abstract val sdkDirPath: Property<String>

    @get:OutputFile
    abstract val metadataOut: RegularFileProperty

    @TaskAction
    fun emit() {
        val apk = apkFile.orNull?.asFile
        if (apk == null || !apk.exists()) {
            logger.warn("emitBuildMetadata: APK not found at ${apk?.absolutePath}; skipping.")
            return
        }

        fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

        fun sha256OfFile(file: File): String {
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                val buffer = ByteArray(64 * 1024)
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    digest.update(buffer, 0, read)
                }
            }
            return hex(digest.digest())
        }

        fun signingCertSha256FromV1(file: File): String? {
            ZipFile(file).use { zip ->
                val sigEntry =
                    zip.entries().toList().firstOrNull { entry ->
                        val name = entry.name.uppercase()
                        name.startsWith("META-INF/") &&
                            (name.endsWith(".RSA") || name.endsWith(".DSA") || name.endsWith(".EC"))
                    } ?: return null
                val pkcs7Bytes = zip.getInputStream(sigEntry).use { it.readBytes() }
                val factory = CertificateFactory.getInstance("X.509")
                val certs = factory.generateCertificates(pkcs7Bytes.inputStream())
                val cert = certs.firstOrNull() ?: return null
                return hex(MessageDigest.getInstance("SHA-256").digest(cert.encoded))
            }
        }

        fun resolveApksigner(): File? {
            val sdkDirString = sdkDirPath.orNull ?: return null
            val sdkDir = File(sdkDirString)
            if (!sdkDir.isDirectory) return null
            val buildTools = File(sdkDir, "build-tools")
            if (!buildTools.isDirectory) return null
            return buildTools.listFiles { f -> f.isDirectory }
                ?.sortedByDescending { it.name }
                ?.firstNotNullOfOrNull { dir -> File(dir, "apksigner").takeIf { it.canExecute() } }
        }

        fun signingCertSha256ViaApksigner(file: File): String? {
            val apksigner = resolveApksigner() ?: return null
            return try {
                val process =
                    ProcessBuilder(apksigner.absolutePath, "verify", "--print-certs", file.absolutePath)
                        .redirectErrorStream(true).start()
                val output = process.inputStream.bufferedReader().use { it.readText() }
                process.waitFor()
                if (process.exitValue() != 0) return null
                output.lineSequence()
                    .firstOrNull { line -> line.contains("certificate SHA-256 digest:") }
                    ?.substringAfterLast(':')
                    ?.trim()
                    ?.lowercase()
            } catch (_: Exception) {
                null
            }
        }

        fun signingCertSha256(file: File): String? =
            signingCertSha256ViaApksigner(file) ?: signingCertSha256FromV1(file)

        fun jsonString(value: String?): String {
            if (value == null) return "null"
            val sb = StringBuilder("\"")
            for (ch in value) {
                when (ch) {
                    '\\' -> sb.append("\\\\")
                    '"' -> sb.append("\\\"")
                    '\n' -> sb.append("\\n")
                    '\r' -> sb.append("\\r")
                    '\t' -> sb.append("\\t")
                    else -> sb.append(ch)
                }
            }
            sb.append("\"")
            return sb.toString()
        }

        val apkSha256 = sha256OfFile(apk)
        val certSha256 = signingCertSha256(apk)
        val repoRoot = File(repoRootPath.get())
        val relPath = repoRoot.toPath().relativize(apk.toPath()).toString()

        val json = buildString {
            append("{\n")
            append("  \"package\": ${jsonString(packageName.get())},\n")
            append("  \"version_code\": ${versionCode.get()},\n")
            append("  \"version_name\": ${jsonString(versionName.get())},\n")
            append("  \"apk_relative_path\": ${jsonString(relPath)},\n")
            append("  \"apk_sha256\": ${jsonString(apkSha256)},\n")
            append("  \"signing_cert_sha256\": ${jsonString(certSha256 ?: "")}\n")
            append("}\n")
        }
        val outFile = metadataOut.get().asFile
        outFile.writeText(json)
        logger.lifecycle("emitBuildMetadata: wrote ${outFile.absolutePath}")
    }
}

android {
    namespace = "com.skycua.phonecompanion"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.skycua.phonecompanion"
        // AGSL RuntimeShader (the agent-overlay smoke/glow renderer) requires
        // API 33. Older devices (Android 11-12) are dropped intentionally.
        minSdk = 33
        targetSdk = 36
        versionCode = 2
        versionName = "0.1.1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // Expose version metadata to runtime code explicitly. Newer AGP no
        // longer guarantees VERSION_CODE/VERSION_NAME in BuildConfig. Derive
        // these from the versionCode/versionName above so the runtime-reported
        // version can never drift from the manifest (the companion surfaces
        // BuildConfig.VERSION_NAME as its installed version).
        buildConfigField("int", "VERSION_CODE", "$versionCode")
        buildConfigField("String", "VERSION_NAME", "\"$versionName\"")
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The build runs on JDK 21 (the AGP-supported toolchain in this
    // environment); bytecode target stays at 17 for broad device compatibility.
    kotlin {
        jvmToolchain(21)
    }

    buildFeatures {
        buildConfig = true
    }

    androidResources {
        // Keep the Opus easter-egg sound stored uncompressed so SoundPool can open
        // it via a raw resource file descriptor; AAPT compresses .opus by default.
        noCompress += "opus"
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
        unitTests.isIncludeAndroidResources = true
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.appcompat)
    implementation(libs.androidx.annotation)
    implementation(libs.okhttp)
    implementation(libs.camera.core)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.video)
    implementation(libs.camera.view)
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
}

/**
 * Emits build metadata consumed by the sky-cua host for companion identity and
 * install policy (see docs/runtime/phone-companion-protocol.md). Writes package
 * id, versionCode, versionName, the APK relative path, the APK SHA-256, and the
 * signing certificate SHA-256 to android/phone-companion/build-metadata.json.
 *
 * The certificate fingerprint is read from the actual built APK so it always
 * reflects the signer that produced the artifact (debug keystore for local
 * sideloading), matching the host signature-comparison contract.
 *
 * This task is configuration-cache compatible: all inputs are declared as
 * isolated properties (no Project references leak into the action).
 */
val emitBuildMetadata =
    tasks.register<EmitBuildMetadataTask>("emitBuildMetadata") {
        description = "Writes build-metadata.json for the sky-cua host."
        group = "sky-cua"

        packageName.set(android.defaultConfig.applicationId ?: "com.skycua.phonecompanion")
        versionCode.set(android.defaultConfig.versionCode ?: 0)
        versionName.set(android.defaultConfig.versionName ?: "")
        apkFile.set(layout.buildDirectory.file("outputs/apk/debug/app-debug.apk"))
        repoRootPath.set(rootProject.projectDir.parentFile.parentFile.absolutePath)
        metadataOut.set(File(rootProject.projectDir, "build-metadata.json"))

        // Resolve SDK dir at configuration time so the action remains cache-compatible
        // (no Project/System.getenv lookups in the isolated action beyond the captured string).
        val sdkDir: File? =
            System.getenv("ANDROID_SDK_ROOT")?.let { File(it) }
                ?: System.getenv("ANDROID_HOME")?.let { File(it) }
                ?: File(rootProject.projectDir, "local.properties").takeIf { it.isFile }?.let { props ->
                    props.readLines()
                        .firstOrNull { it.trimStart().startsWith("sdk.dir=") }
                        ?.substringAfter("sdk.dir=")
                        ?.trim()
                        ?.let { File(it) }
                }
        if (sdkDir != null) sdkDirPath.set(sdkDir.absolutePath)

        // Declare that APK is optional: older Gradle would use `inputs.files(...).optional()`; with
        // typed properties `@Optional` + `RegularFileProperty` already expresses this.
    }

tasks.matching { it.name == "assembleDebug" }.configureEach {
    finalizedBy(emitBuildMetadata)
}
