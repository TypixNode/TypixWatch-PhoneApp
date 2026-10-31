plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

val abis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")
val minApi = 26
val core = rootProject.file("../core")
val generated = layout.buildDirectory.dir("generated/typix")
val jniOut = generated.map { it.dir("jniLibs") }
val kotlinOut = generated.map { it.dir("kotlin") }

android {
    namespace = "com.typixnode.watch"
    compileSdk = 36
    ndkVersion = "27.1.12297006"

    defaultConfig {
        applicationId = "com.typixnode.watch"
        minSdk = minApi
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += abis }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

/** cargo is not on the PATH Gradle's daemon inherits from an IDE. */
fun Exec.rustEnvironment() {
    environment("PATH", "${System.getProperty("user.home")}/.cargo/bin:/opt/homebrew/bin:${System.getenv("PATH")}")
}

/** Everything that changes the Rust output. */
fun Task.rustInputs() {
    inputs.dir(core.resolve("src"))
    inputs.dir(core.resolve("vendor"))
    inputs.files(
        core.resolve("Cargo.toml"),
        core.resolve("Cargo.lock"),
        core.resolve("build.rs"),
        core.resolve(".cargo/config.toml"),
    )
}

// Rust to libtypix_core.so for each ABI, unstripped: the next task reads the UniFFI
// metadata out of it, and a crash needs its line tables.  AGP strips the APK's copy.
val cargoNdk by tasks.registering(Exec::class) {
    group = "rust"
    description = "Builds core/ for every ABI into build/generated/typix/jniLibs"
    rustInputs()
    outputs.dir(jniOut)
    workingDir = core
    rustEnvironment()
    environment("ANDROID_NDK_HOME", androidComponents.sdkComponents.ndkDirectory.get().asFile.path)
    commandLine(
        listOf("cargo", "ndk") + abis.flatMap { listOf("-t", it) } +
            listOf("-P", "$minApi", "-o", jniOut.get().asFile.path) +
            listOf("build", "--release", "--lib"),
    )
}

// The bindings describe the library just built: UniFFI reads the metadata out of the .so.
val uniffiBindings by tasks.registering(Exec::class) {
    group = "rust"
    description = "Generates the Kotlin bindings into build/generated/typix/kotlin"
    dependsOn(cargoNdk)
    rustInputs()
    outputs.dir(kotlinOut)
    workingDir = core
    rustEnvironment()
    commandLine(
        "cargo", "run", "--quiet", "--features", "bindgen", "--bin", "uniffi-bindgen", "--",
        "generate", "--no-format", "--language", "kotlin",
        "--library", jniOut.get().file("arm64-v8a/libtypix_core.so").asFile.path,
        "--out-dir", kotlinOut.get().asFile.path,
    )
}

android.sourceSets.getByName("main") {
    kotlin.directories.add(kotlinOut.get().asFile.path)
    jniLibs.directories.add(jniOut.get().asFile.path)
}

tasks.named("preBuild") { dependsOn(uniffiBindings) }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.12.00"))
    implementation("androidx.activity:activity-compose:1.12.2")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    // The UniFFI bindings load the Rust library through JNA.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
}
