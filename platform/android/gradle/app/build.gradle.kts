plugins {
    id("com.android.application")
}

val repositoryAndroidTarget = rootProject.layout.projectDirectory.dir("../../../target/android")
layout.buildDirectory = repositoryAndroidTarget.dir("gradle/app")

android {
    namespace = "com.flectar.mail"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.flectar.mail"
        minSdk = 26
        targetSdk = 36
        versionCode = providers.environmentVariable("FLECTAR_ANDROID_VERSION_CODE")
            .orElse("1").get().toInt()
        versionName = providers.environmentVariable("FLECTAR_ANDROID_VERSION_NAME")
            .orElse("0.1.0").get()
    }

    sourceSets["main"].jniLibs.srcDir(repositoryAndroidTarget.dir("gradle-jni"))
    sourceSets["main"].res.srcDir(layout.buildDirectory.dir("generated/flectar-res"))
    sourceSets["main"].assets.srcDirs(
        "../../assets",
        layout.buildDirectory.dir("generated/flectar-assets"),
    )

    signingConfigs {
        create("releaseFromEnvironment") {
            val keystore = providers.environmentVariable("FLECTAR_ANDROID_KEYSTORE").orNull
            if (!keystore.isNullOrBlank()) {
                storeFile = file(keystore)
                storePassword = providers.environmentVariable("FLECTAR_ANDROID_KEYSTORE_PASSWORD").get()
                keyAlias = providers.environmentVariable("FLECTAR_ANDROID_KEY_ALIAS").get()
                keyPassword = providers.environmentVariable("FLECTAR_ANDROID_KEY_PASSWORD").get()
            }
        }
    }

    buildTypes {
        getByName("release") {
            isDebuggable = false
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("releaseFromEnvironment")
        }
    }
}

val generateFlectarLicenseAssets by tasks.registering(Copy::class) {
    into(layout.buildDirectory.dir("generated/flectar-assets/licenses/flectar-mail"))
    from("../../../../LICENSE")
    from("../../../../THIRD_PARTY_NOTICES.md")
    from("../../../../LICENSES") {
        into("LICENSES")
    }
}

tasks.named("preBuild").configure {
    dependsOn(generateFlectarLicenseAssets)
}

dependencies {
    // AuthorizationClient is Google's supported API for authorizing an
    // Android application to Gmail and other Google user data.
    implementation("com.google.android.gms:play-services-auth:21.6.0")
}
