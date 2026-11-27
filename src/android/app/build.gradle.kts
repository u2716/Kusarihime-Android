plugins { id("com.android.application"); id("org.jetbrains.kotlin.android") }
android {
    namespace = "com.fujihime.euthanasia"
    compileSdk = 34
    defaultConfig {
        applicationId = "com.fujihime.euthanasia"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    buildTypes { release { isMinifyEnabled = false; isShrinkResources = false } }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    kotlinOptions { jvmTarget = "17" }
    packaging { jniLibs { useLegacyPackaging = true } }
    // Rust .so is built by cargo-ndk (scripts/build-android.sh) into app/src/main/jniLibs/<abi>/.
    // If FUJIHIME_SKIP_RUST=1, Gradle will use whatever .so is already there.
}
dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.documentfile:documentfile:1.0.1")
}
