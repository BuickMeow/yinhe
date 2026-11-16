# Rust 项目 target 瘦身指南

> 适用于任何 Rust 项目，不限于 yinhe。目标是**大幅减少磁盘占用，同时不明显拖慢编译**。
> 文末附 yinhe 的实测数据（27G → 10G）。

## 0. 先搞清楚空间被谁吃了

不要凭感觉删。先量：

```sh
du -sh target
du -sh target/* | sort -h          # debug / release / 各交叉编译目标分别多大
du -sh target/debug/* | sort -h    # incremental / deps / build / examples
```

关键经验：**`target/debug` 通常是唯一的巨无霸**，其中两部分最容易被忽视：

```sh
# 1) 增量编译缓存（改一次代码就多一份，旧版本永远不会被自动回收）
du -sh target/debug/incremental
ls target/debug/incremental | sed 's/-[0-9a-z]*$//' | sort | uniq -c | sort -rn | head
#    同一个 crate 出现几十次 = rustc 升级 / feature 变化后的僵尸缓存

# 2) macOS 默认 split-debuginfo="unpacked" 留下的目标文件
find target/debug/deps -maxdepth 1 -name '*.o' -exec du -k {} + \
  | awk '{s+=$1} END {printf "%.2f GB in %d files\n", s/1048576, NR}'
```

先预演清理范围，确认不会误伤 release：

```sh
cargo clean --profile dev --dry-run     # 较新 cargo 支持；旧版只能 rm -rf target/debug
cargo clean --target <triple> --dry-run
```

## 1. 收益最大的配置：少生成 debuginfo

调试信息（DWARF）通常占 `target/debug` 的 40%～60%，而生成它本身也很费编译时间。**砍掉它既省空间又更快**，这是唯一无痛的大招。

```toml
# Cargo.toml
[profile.dev]
debug = 1                 # 只保留行号表：panic backtrace 仍有文件名:行号

[profile.dev.package."*"]
debug = false             # 第三方依赖完全不生成 debuginfo
```

要点：

- `debug = 1` 与 `debug = true`（=2）的差别是变量、类型、内联信息；**行号保留**，所以 backtrace、profiler 的火焰图仍然可用。
- 不给依赖生成 debuginfo 的代价：不能单步进第三方源码看变量。日常开发几乎用不到；真要调库时临时改回 `debug = 2` 重编那一个依赖即可。
- `[profile.dev.package."*"]` 只作用于依赖，自己的 crate 不受影响。
- `profile.test` 继承 `dev`，所以 `cargo test` 同样受益。

预期效果：debug 目录体积降到原来的 **1/3 左右**，clippy/test 反而更快。

如果项目有自定义 profile（如 `fast-release`），按同样思路单独检查，别只改 `dev`。

## 2. 清理历史增量缓存

`incremental` 目录只对“反复修改同一 crate”有用，且旧 fingerprint 永远不会被自动回收。定期删一次，代价只是下一次编译慢一点：

```sh
rm -rf target/debug/incremental
# 或精确一点：
cargo clean --profile dev
```

**不要**为了省空间全局关掉增量编译：

```toml
# 不推荐
[profile.dev]
incremental = false
```

只在 CI / 打包脚本里用环境变量临时关闭：

```sh
CARGO_INCREMENTAL=0 cargo build --release
```

## 3. 交叉编译目标分开放

Android、OHOS、wasm 等目标默认和宿主编译共享同一个 `target`，动辄几个 G，却没法单独清理。按目标拆目录：

```sh
CARGO_TARGET_DIR=target/android cargo build --target aarch64-linux-android
CARGO_TARGET_DIR=target/ohos    cargo build --target aarch64-unknown-linux-ohos
```

好处不是总量变少，而是能单独清：

```sh
cargo clean --target aarch64-unknown-linux-ohos
rm -rf target/android          # 交叉目录直接删最省事
```

打包脚本（xtask / Makefile / CI）里固定好这几个变量，别每次手敲。

## 4. 排查重复依赖版本

同一个 crate 存在多个版本会让依赖图凭空膨胀几十个包。先统计，再决定动不动手。

### 4.1 注意：`Cargo.lock` 不等于本机真的编译

`Cargo.lock` 里有大量 `cfg(windows)`、`cfg(android)` 的条件依赖，直接看会被误导。先用 `cargo tree` 拿到本机真实的包集合，再做交集：

```sh
cargo tree --workspace -e normal,build,dev --prefix none --format '{p}' \
  | sort -u > /tmp/host-pkgs.txt
```

### 4.2 统计脚本

```sh
python3 - <<'EOF'
import re, collections

# 1) 解析 Cargo.lock：同名多版本
blocks = open('Cargo.lock').read().split('[[package]]')[1:]
g = collections.defaultdict(list)
for b in blocks:
    n = re.search(r'^name = "(.*)"', b, re.M)
    v = re.search(r'^version = "(.*)"', b, re.M)
    if n and v:
        g[n.group(1)].append(v.group(1))

# 2) 只保留本机真实参与编译的包
onhost = set()
for line in open('/tmp/host-pkgs.txt'):
    m = re.match(r'(.*) v([^ ]+)', line.strip())
    if m:
        onhost.add((m.group(1), m.group(2)))

dups = {k: v for k, v in g.items() if len(v) > 1}
real = [(k, [v for v in vs if (k, v) in onhost])
        for k, vs in dups.items()
        if len([v for v in vs if (k, v) in onhost]) > 1]

print(f'lock 级重复: {len(dups)}，本机真实重复: {len(real)}')
for k, on in sorted(real):
    print(f'{k:24s} {" | ".join(on)}')
EOF
```

### 4.3 找引入者，判断能不能动

```sh
cargo tree -i 'syn@1.0.109' --workspace -e normal,build,dev --depth 1 --prefix none
```

处理原则：

- **引入者已是最新版本** → 动不了，不要为了合并版本去 `[patch.crates-io]` 强推 semver 不兼容的版本，坏掉的风险远大于省下的几十 MB。
- **引入者自己可以升级，且新版换用了新依赖** → 升级。
- **自己 workspace 里声明的版本落后**（如 zstd 0.13 而生态已 0.14）→ 直接升，顺带合并。
- 升级后必须重新跑一遍 4.2，确认收益是净正（有时升级会引入新的重复，见下）。

反例（yinhe 实测）：`unarc-rs 0.6 → 0.8` 消掉了整套 crypto 重复（sha2/aes/cipher/digest/... 9 个），但同时引入了 `lzma-rust2 0.23/0.24` 新的一对；把测试用 `zip 8 → 9` 和 `zstd 0.13 → 0.14` 一起升掉后才是净收益。

## 5. 不要做的事

| 做法 | 为什么不要 |
| --- | --- |
| 全局 `incremental = false` | 改一个 crate 就要全量重编，日常迭代变慢最明显的一项 |
| `split-debuginfo = "packed"` | macOS 默认 `unpacked` 就是为了链接快；改成 packed 能省掉几 G 的 `.o`，但链接明显变慢 |
| `[patch.crates-io]` 强行合并大版本 | semver 不兼容，随时炸；省下的空间不值这个风险 |
| 删除 `target/release` 换 debug 体积 | release 是你实际要跑的东西，重编动辄十几分钟 |
| 只看 `Cargo.lock` 就动手 | 一半重复来自其他平台，改了对本机毫无影响 |

## 6. 标准操作流程（照抄）

```sh
# 1. 量
du -sh target/*; du -sh target/debug/*

# 2. 改 Cargo.toml（第 1 节的两段 profile）

# 3. 清旧产物（旧产物不会被新配置自动替换，必须清一次）
cargo clean --profile dev --dry-run    # 先确认
cargo clean --profile dev

# 4. 重建 + 验证
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --release

# 5. 再量一次
du -sh target
```

注意第 3 步：**只改配置不清旧产物是没用的**。cargo 按 fingerprint 增删文件，旧的高体积产物会一直留在 `deps` 里。

## 7. yinhe 实测记录

改动前：

| 位置 | 大小 |
| --- | --- |
| `target/debug/incremental` | 8.1G（含 35 份同 crate 的僵尸缓存） |
| `target/debug/deps/*.o` | 6.5G / 43096 个文件 |
| `target/debug/deps/*.rlib` | 3.7G |
| `target/release` | 2.9G |
| OHOS + Android | 6.2G |
| **合计** | **27G** |

做过的事：

1. `[profile.dev] debug = 1` + `[profile.dev.package."*"] debug = false`
2. `cargo clean --profile dev` 清掉旧 full-DWARF 产物（15.1G）
3. `unarc-rs 0.6 → 0.8.1`，顺手升 `zip 8.6 → 9`、`zstd 0.13 → 0.14`

结果：

| 指标 | 前 | 后 |
| --- | --- | --- |
| `target` 合计 | 27G | 10G |
| 本机真实重复包 | 26 个 | 19 个 |
| lock 级重复包 | 65 个 | 58 个 |

剩余 19 个重复全部由上游锁死（egui / winit / xsynth / rust-i18n / wgpu），等它们自己迁移即可。

## 8. 顺带收益

升级依赖时留意一下 C++ 依赖：yinhe 的 `unarc-rs 0.8` 改用纯 Rust 的 RAR 实现后，`unrar_sys` 从依赖树里彻底消失。这类变化能让交叉编译链路更简单（例如 Android 不再需要 `-lc++_shared`），但**必须实测后再删对应的链接参数**。
