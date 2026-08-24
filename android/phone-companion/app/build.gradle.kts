import java.io.File
import java.security.MessageDigest
import java.security.cert.CertificateFactory
import java.util.zip.ZipFile

plugins {
    alias(libs.plugins.android.application)
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
 */
val emitBuildMetadata =
    tasks.register("emitBuildMetadata") {
        description = "Writes build-metadata.json for the sky-cua host."
        group = "sky-cua"

        val pkg = android.defaultConfig.applicationId ?: "com.skycua.phonecompanion"
        val versionCode = android.defaultConfig.versionCode ?: 0
        val versionName = android.defaultConfig.versionName ?: ""
        val apkFile = layout.buildDirectory.file("outputs/apk/debug/app-debug.apk")
        val repoRoot = rootProject.projectDir.parentFile.parentFile
        val metadataOut = File(rootProject.projectDir, "build-metadata.json")

        inputs.files(apkFile).optional()
        outputs.file(metadataOut)

        doLast {
            val apk = apkFile.get().asFile
            if (!apk.exists()) {
                logger.warn("emitBuildMetadata: APK not found at ${apk.absolutePath}; skipping.")
                return@doLast
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

            // Reads the v1 (JAR) signing certificate from META-INF, used as a
            // fallback when apksigner is unavailable. Modern APKs are signed with
            // v2/v3 schemes and may not carry a v1 block, so this can return null.
            fun signingCertSha256FromV1(file: File): String? {
                ZipFile(file).use { zip ->
                    val sigEntry =
                        zip.entries().toList().firstOrNull { entry ->
                            val name = entry.name.uppercase()
                            name.startsWith("META-INF/") &&
                                (
                                    name.endsWith(".RSA") ||
                                        name.endsWith(".DSA") ||
                                        name.endsWith(".EC")
                                )
                        } ?: return null
                    val pkcs7Bytes = zip.getInputStream(sigEntry).use { it.readBytes() }
                    val factory = CertificateFactory.getInstance("X.509")
                    val certs = factory.generateCertificates(pkcs7Bytes.inputStream())
                    val cert = certs.firstOrNull() ?: return null
                    return hex(MessageDigest.getInstance("SHA-256").digest(cert.encoded))
                }
            }

            // Resolves the newest apksigner from the SDK build-tools so the cert
            // fingerprint reflects the v2/v3 signer that actually signed the APK.
            fun sdkDirFromLocalProperties(): File? {
                val props = File(rootProject.projectDir, "local.properties")
                if (!props.isFile) return null
                return props
                    .readLines()
                    .firstOrNull { it.trimStart().startsWith("sdk.dir=") }
                    ?.substringAfter("sdk.dir=")
                    ?.trim()
                    ?.let { File(it) }
            }

            fun resolveApksigner(): File? {
                val sdkDir =
                    System.getenv("ANDROID_SDK_ROOT")?.let { File(it) }
                        ?: System.getenv("ANDROID_HOME")?.let { File(it) }
                        ?: sdkDirFromLocalProperties()
                        ?: return null
                val buildTools = File(sdkDir, "build-tools")
                if (!buildTools.isDirectory) return null
                return buildTools
                    .listFiles { f -> f.isDirectory }
                    ?.sortedByDescending { it.name }
                    ?.firstNotNullOfOrNull { dir ->
                        File(dir, "apksigner").takeIf { it.canExecute() }
                    }
            }

            fun signingCertSha256ViaApksigner(file: File): String? {
                val apksigner = resolveApksigner() ?: return null
                return try {
                    val process =
                        ProcessBuilder(
                            apksigner.absolutePath,
                            "verify",
                            "--print-certs",
                            file.absolutePath,
                        ).redirectErrorStream(true).start()
                    val output = process.inputStream.bufferedReader().use { it.readText() }
                    process.waitFor()
                    if (process.exitValue() != 0) return null
                    output
                        .lineSequence()
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
            val relPath = repoRoot.toPath().relativize(apk.toPath()).toString()

            val json =
                buildString {
                    append("{\n")
                    append("  \"package\": ${jsonString(pkg)},\n")
                    append("  \"version_code\": $versionCode,\n")
                    append("  \"version_name\": ${jsonString(versionName)},\n")
                    append("  \"apk_relative_path\": ${jsonString(relPath)},\n")
                    append("  \"apk_sha256\": ${jsonString(apkSha256)},\n")
                    append("  \"signing_cert_sha256\": ${jsonString(certSha256 ?: "")}\n")
                    append("}\n")
                }
            metadataOut.writeText(json)
            logger.lifecycle("emitBuildMetadata: wrote ${metadataOut.absolutePath}")
        }
    }

tasks.matching { it.name == "assembleDebug" }.configureEach {
    finalizedBy(emitBuildMetadata)
}
