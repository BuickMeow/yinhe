plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.jieneng.yinhe_flutter"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.jieneng.yinhe_flutter"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

// ── cargo-ndk 构建任务：Rust → libyinhe_flutter.so → jniLibs ──
// 与仓库 android/app/build.gradle.kts 的同类任务保持一致：
// NDK r29 的 sysroot 主目录只有静态 .a（静态链接的 libc 与部分设备不兼容），
// 动态库在版本化子目录 35/ 里，链接器搜不到；这里建符号链接让
// -lc/-lm/-ldl/-llog/-landroid/-laaudio/-lpthread 动态解析到设备侧 libc。
tasks.register<Exec>("ensureSysrootLibs") {
    val ndkRoot = System.getenv("ANDROID_NDK_HOME")
        ?: File(
            System.getenv("ANDROID_HOME") ?: "${System.getProperty("user.home")}/Library/Android/sdk",
            "ndk"
        ).listFiles()?.maxByOrNull { it.name }?.absolutePath
        ?: error("NDK not found: set ANDROID_NDK_HOME or ANDROID_HOME")
    val prebuilt =
        File(ndkRoot, "toolchains/llvm/prebuilt").listFiles()?.first()?.absolutePath
            ?: error("NDK prebuilt dir not found in $ndkRoot")
    val libDir = File(prebuilt, "sysroot/usr/lib/aarch64-linux-android")
    workingDir(libDir)
    commandLine(
        "sh", "-c",
        "for lib in libc libm libdl liblog libandroid libaaudio; do " +
            "ln -sf 35/\$lib.so \$lib.so; " +
            "rm -f \$lib.a; done; " +
            "ln -sf 35/libc.so libpthread.so; rm -f libpthread.a"
    )
}

tasks.register<Exec>("cargoNdkBuild") {
    // workingDir = 仓库根（flutter/android/app 的上三级）。
    workingDir(rootProject.projectDir.parentFile.parentFile)
    val outDir = project.layout.projectDirectory.dir("src/main/jniLibs").asFile.absolutePath
    // NDK 路径由 cargo-ndk 自动探测（ANDROID_HOME/默认 SDK 路径）。
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "-P", "35",
        "-o", outDir,
        "build", "--release", "-p", "yinhe-flutter"
    )
}

tasks.named("cargoNdkBuild") {
    dependsOn("ensureSysrootLibs")
}

tasks.named("preBuild") {
    dependsOn("cargoNdkBuild")
}
