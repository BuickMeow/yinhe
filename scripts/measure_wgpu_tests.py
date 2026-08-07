#!/usr/bin/env python3
"""逐个运行 yinhe-wgpu 的每个测试并计时，找出耗时大头。单个超时 90s 截断。"""
import subprocess, time

out = subprocess.run(
    ["cargo", "test", "-p", "yinhe-wgpu", "--", "--list"],
    capture_output=True, text=True, cwd=".",
).stdout
tests = [l[: -len(": test")] for l in out.splitlines() if l.endswith(": test")]

results = []
for t in tests:
    s = time.time()
    try:
        r = subprocess.run(
            ["cargo", "test", "-p", "yinhe-wgpu", t, "--", "--exact", "--test-threads=1"],
            capture_output=True, text=True, cwd=".", timeout=90,
        )
        status = "PASS" if r.returncode == 0 else "FAIL"
    except subprocess.TimeoutExpired:
        status = "TIMEOUT"
    e = time.time()
    results.append((e - s, t, status))
    print(f"{e-s:7.2f}s  {status}  {t}", flush=True)

results.sort(reverse=True)
print("\n=== Top 20 耗时 ===")
for sec, name, status in results[:20]:
    print(f"{sec:7.2f}s  {status}  {name}")
total = sum(t for t, _, _ in results)
print(f"\n测试数={len(results)} 总耗时={total:.1f}s (含每次 cargo 启动约 0.4s)")
