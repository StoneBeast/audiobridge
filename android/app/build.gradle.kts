plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.audiobridge.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.audiobridge.app"
        minSdk = 29
        targetSdk = 34
        versionCode = 4
        versionName = "0.3.1"
    }

    signingConfigs {
        // 发行签名（若存在）：生成方式见 docs/build-android.md
        val ksFile = rootProject.file("keystore/release.jks")
        if (ksFile.exists()) {
            create("release") {
                storeFile = ksFile
                storePassword = (project.findProperty("AB_STORE_PASSWORD") as String?) ?: "audiobridge"
                keyAlias = "audiobridge"
                keyPassword = (project.findProperty("AB_KEY_PASSWORD") as String?) ?: "audiobridge"
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (rootProject.file("keystore/release.jks").exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    testOptions {
        // JVM 单元测试中 android.util.Log 等框架方法返回默认值而非抛异常
        unitTests.isReturnDefaultValues = true
        // JVM 单元测试读取仓库根目录 protocol/ 下的一致性测试向量
        sourceSets["test"].resources.srcDir(rootDir.resolve("../protocol"))
    }
    packaging {
        resources.excludes += "META-INF/{AL2.0,LGPL2.1}"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.6")
    implementation("androidx.activity:activity-compose:1.9.2")
    implementation(platform("androidx.compose:compose-bom:2024.09.03"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    testImplementation("junit:junit:4.13.2")
    // JVM 单元测试解析一致性向量 JSON 用（android.jar 不含 org.json 的实现）
    testImplementation("org.json:json:20240303")
}
