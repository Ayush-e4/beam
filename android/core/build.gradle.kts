// The Rust engine (core/beam-ffi) for Android: builds libbeam_ffi.so for each ABI with cargo-ndk
// and generates its Kotlin bindings (package app.beam.core) with UniFFI. Nothing generated is
// checked in; both land in build/ and are wired into every variant.

import javax.inject.Inject

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.ktlint)
}

/** The Rust workspace, next to the Android project. */
val rustWorkspace: Directory = rootProject.layout.projectDirectory.dir("../core")

/** ABIs to build; `-Pbeam.rustAbis=arm64-v8a` builds just one while iterating locally. */
val rustAbis: List<String> =
    providers.gradleProperty("beam.rustAbis").orNull?.split(",")?.map(String::trim)
        ?: listOf("arm64-v8a", "armeabi-v7a", "x86_64")

/** rustup's cargo, which IDE-started Gradle daemons often cannot find on PATH. */
val cargoExecutable: String =
    providers.gradleProperty("beam.cargo").orNull
        ?: File(System.getProperty("user.home"), ".cargo/bin/cargo").takeIf {
            it.canExecute()
        }?.path
        ?: "cargo"

android {
    namespace = "app.beam.core"
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }
    ndkVersion = libs.versions.ndk.get()

    defaultConfig {
        minSdk = 26
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        consumerProguardFiles("consumer-rules.pro")
    }

    lint {
        warningsAsErrors = true
        abortOnError = true
    }
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        allWarningsAsErrors.set(true)
    }
}

ktlint {
    // UniFFI's generated bindings are not ours to format.
    filter { exclude { it.file.path.contains("/build/") } }
}

/** Cross-compiles beam-ffi in release mode into a jniLibs layout. */
abstract class CargoNdkBuild @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
    @get:Input abstract val cargo: Property<String>

    @get:Input abstract val abis: ListProperty<String>

    @get:Input abstract val minSdk: Property<Int>

    @get:Internal abstract val workspace: DirectoryProperty

    @get:Internal abstract val ndkDirectory: DirectoryProperty

    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    init {
        // Cargo knows when Rust sources changed and is a quick no-op otherwise.
        outputs.upToDateWhen { false }
    }

    @TaskAction
    fun build() {
        exec.exec {
            workingDir(workspace)
            executable(cargo.get())
            args("ndk", "--platform", minSdk.get().toString())
            abis.get().forEach { args("--target", it) }
            args("--output-dir", outputDir.get().asFile.path)
            args("build", "--release", "--locked", "--package", "beam-ffi")
            environment("ANDROID_NDK_HOME", ndkDirectory.get().asFile.path)
        }
    }
}

/** Generates the Kotlin bindings from a built library, which carries UniFFI's metadata. */
abstract class UniffiBindings @Inject constructor(
    private val exec: ExecOperations,
    private val files: FileSystemOperations
) : DefaultTask() {
    @get:Input abstract val cargo: Property<String>

    @get:Internal abstract val workspace: DirectoryProperty

    @get:InputFiles abstract val libraries: DirectoryProperty

    @get:Input abstract val abi: Property<String>

    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        files.delete { delete(outputDir) }
        val library = libraries.get().file("${abi.get()}/libbeam_ffi.so").asFile
        exec.exec {
            workingDir(workspace)
            executable(cargo.get())
            args("run", "--quiet", "--locked", "--package", "uniffi-bindgen", "--")
            args("generate", "--no-format", "--language", "kotlin")
            args("--library", library.path, "--out-dir", outputDir.get().asFile.path)
        }
    }
}

/** Builds beam-ffi for this computer, so JVM unit tests can load it through JNA. */
abstract class CargoHostBuild @Inject constructor(private val exec: ExecOperations) :
    DefaultTask() {
    @get:Input abstract val cargo: Property<String>

    @get:Internal abstract val workspace: DirectoryProperty

    init {
        outputs.upToDateWhen { false }
    }

    @TaskAction
    fun build() {
        exec.exec {
            workingDir(workspace)
            executable(cargo.get())
            args("build", "--locked", "--package", "beam-ffi")
        }
    }
}

val rustLibraries =
    tasks.register<CargoNdkBuild>("cargoNdkBuild") {
        cargo.set(cargoExecutable)
        abis.set(rustAbis)
        minSdk.set(android.defaultConfig.minSdk ?: 26)
        workspace.set(rustWorkspace)
        ndkDirectory.set(androidComponents.sdkComponents.ndkDirectory)
        outputDir.set(layout.buildDirectory.dir("rust/jniLibs"))
    }

val kotlinBindings =
    tasks.register<UniffiBindings>("uniffiBindings") {
        cargo.set(cargoExecutable)
        workspace.set(rustWorkspace)
        libraries.set(rustLibraries.flatMap { it.outputDir })
        abi.set(rustAbis.first())
        outputDir.set(layout.buildDirectory.dir("generated/uniffi"))
    }

val hostLibrary =
    tasks.register<CargoHostBuild>("cargoHostBuild") {
        cargo.set(cargoExecutable)
        workspace.set(rustWorkspace)
    }

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(
            rustLibraries,
            CargoNdkBuild::outputDir
        )
        variant.sources.kotlin?.addGeneratedSourceDirectory(
            kotlinBindings,
            UniffiBindings::outputDir
        )
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(hostLibrary)
    systemProperty("jna.library.path", rustWorkspace.dir("target/debug").asFile.path)
}

dependencies {
    // The AAR carries JNA's Android native part; tests on the JVM need the plain JAR instead.
    implementation(libs.jna) { artifact { type = "aar" } }
    // UniFFI turns the engine's async calls into suspend functions.
    api(libs.kotlinx.coroutines.core)
    // @RequiresApi for the Android 14+ discovery path.
    implementation(libs.androidx.annotation)
    testImplementation(libs.jna)
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    // Run on a connected phone: ./gradlew :core:connectedDebugAndroidTest
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.junit)
}
