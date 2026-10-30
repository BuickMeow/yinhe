plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.jieneng.yinhe"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.jieneng.yinhe"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        // AAudio（cpal 的安卓后端）自 API 26 才有，libyinhe_flutter.so 的
        // DT_NEEDED 直接指向 libaaudio.so：minSdk 低于 26 装上也起不来。
        minSdk = 26
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

    packaging {
        jniLibs {
            // cargo-ndk 会把**所有** cdylib crate 的产物都拷进 jniLibs，
            // 于是 sevenz-rust2（crate-type 含 "cdylib"）也带出一份 300KB 的
            // libsevenz_rust2-<hash>.so。它的代码已经静态链进
            // libyinhe_flutter.so，没有任何 DT_NEEDED / dlopen 引用它，纯冗余。
            excludes += "**/libsevenz_rust2-*.so"
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
// `-P` 是**运行时最低 API**，不是编译用的 NDK 版本：链接桩库按该 API 取，
// 生成的 .so 会记录符号版本需求（如 getrandom@LIBC_P = API 28）。
// 曾经用 -P 35 且把顶层 libc.so 软链到 35/，导致 .so 需要
// _Unwind_*@LIBC_R（API 30）→ API 29 及以下（安卓 10 / 鸿蒙 4）加载即失败。
// NDK r29 的 clang 驱动会自己按 --target 找版本化子目录，不需要软链。
//
// 唯一必须补的：rustc 的 android 目标固定传 -lpthread，而 bionic 早已把
// pthread 并进 libc（sysroot 顶层没有 libpthread.so）。软链到目标 API 的
// libc.so 即可 —— 不能链到 35/，否则又退回 LIBC_R(30)。
val ndkApi = "26"

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
    commandLine("sh", "-c", "ln -sf $ndkApi/libc.so libpthread.so")
}

tasks.register<Exec>("cargoNdkBuild") {
    // workingDir = 仓库根（flutter/android/app 的上三级）。
    workingDir(rootProject.projectDir.parentFile.parentFile)
    val outDir = project.layout.projectDirectory.dir("src/main/jniLibs").asFile.absolutePath
    // NDK 路径由 cargo-ndk 自动探测（ANDROID_HOME/默认 SDK 路径）。
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "-P", ndkApi,
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

// AGP 的 native lib 合并任务不保证排在 preBuild 之后；与 cargoNdkBuild
// 并行时可能把上一次的 libyinhe_flutter.so 打进 APK，导致 Dart 新增的
// FFI 符号在运行时找不到（AM 会整块变灰）。强制合并前先完成 Rust 拷贝。
tasks.matching { it.name.matches(Regex("merge.*NativeLibs")) }.configureEach {
    dependsOn("cargoNdkBuild")
}
