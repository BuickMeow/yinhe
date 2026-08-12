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
// NDK r29 的主 sysroot 目录只有静态 .a（libc.a 与部分设备不兼容，静态链接的
// strtod/getpwnam/CPU 特性初始化会在小米等设备上崩溃）。动态库在版本化子目录
// 35/ 里，链接器搜不到。这里在主目录建符号链接指向 35/ 的动态库，让
// -lc/-lm/-ldl/-llog/-landroid/-laaudio/-lpthread 全部动态解析（设备侧 libc）。
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
    // 目标：主目录符号链接 → 版本化目录的动态库（stub，运行期解析到设备 libc）
    commandLine(
        "sh", "-c",
        "for lib in libc libm libdl liblog libandroid libaaudio; do " +
            "ln -sf 35/\$lib.so \$lib.so; " +
            "rm -f \$lib.a; done; " +
            // bionic 无独立 libpthread（符号并入 libc），unrar_sys 的 -lpthread 指向 libc
            "ln -sf 35/libc.so libpthread.so; rm -f libpthread.a"
    )
}

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

tasks.named("cargoNdkBuild") {
    dependsOn("ensureSysrootLibs")
}

tasks.named("preBuild") {
    dependsOn("cargoNdkBuild")
}
