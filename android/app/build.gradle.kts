plugins {
    id("com.android.application")
}

android {
    namespace = "com.jieneng.yinhe"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.jieneng.yinhe"
        // minSdk 26：AAudio（cpal 安卓后端）的最低要求
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    buildTypes {
        release {
            // 阶段 0 先用 debug 签名，正式发布再配置签名
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // GameActivity：软键盘（GameTextInput）+ 生命周期管理。
    // 注意：games-activity 4.4.2 的 pom 漏了传递依赖，appcompat 需显式声明
    //（GameActivity 继承自 AppCompatActivity）。
    implementation("androidx.games:games-activity:4.4.2")
    implementation("androidx.appcompat:appcompat:1.7.1")
}

// ── cargo-ndk 构建任务：Rust → libyinhe_android.so → jniLibs ──
tasks.register<Exec>("cargoNdkBuild") {
    workingDir(rootProject.projectDir.parentFile)
    val outDir = project.layout.projectDirectory.dir("src/main/jniLibs").asFile.absolutePath
    // NDK 路径由 cargo-ndk 自动探测（ANDROID_HOME/默认 SDK 路径）
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "-P", "35",
        "-o", outDir,
        "build", "--release", "-p", "yinhe-android"
    )
}

tasks.named("preBuild") {
    dependsOn("cargoNdkBuild")
}
