// AGP 9.0+ ships built-in Kotlin support (KGP is a runtime dependency of AGP),
// so the standalone org.jetbrains.kotlin.android plugin must not be applied.
plugins {
    alias(libs.plugins.android.application) apply false
}
