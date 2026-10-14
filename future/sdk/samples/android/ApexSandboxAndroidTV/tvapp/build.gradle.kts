plugins {
    id("com.android.application") version "8.5.2"
    kotlin("android") version "1.9.23"
}

android {
    namespace = "com.apex.sandbox.tv"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.apex.sandbox.androidtv"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.leanback:leanback:1.2.0-alpha05")
}
