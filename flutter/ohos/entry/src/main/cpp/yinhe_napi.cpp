//! ArkTS ↔ Rust 的最小 NAPI 桥。
//!
//! 只做一件事：把 Flutter-OH 外接纹理拿到的 `OHNativeWindow*`（bigint）
//! 以及画布尺寸转发给 `libyinhe_flutter.so` 里的 C ABI 入口。
//! 用 dlopen 而不是直接链接：Rust 库由 Flutter 构建流程单独产出，
//! 这样 NAPI 模块和引擎库可以独立替换（调试时只换 .so 即可）。

#include <dlfcn.h>
#include <napi/native_api.h>
#include <cstdint>

namespace {

using InitFn = void (*)(const char*);
using AddSoundfontFn = void (*)(const char*);
using SetSurfaceFn = void (*)(void*);
using ClearSurfaceFn = void (*)();
using SetSizeFn = void (*)(uint32_t, uint32_t);
using SetViewportFn = void (*)(float, float);

void* g_lib = nullptr;

bool EnsureLib() {
  if (g_lib != nullptr) {
    return true;
  }
  g_lib = dlopen("libyinhe_flutter.so", RTLD_NOW | RTLD_GLOBAL);
  return g_lib != nullptr;
}

template <typename Fn>
Fn Sym(const char* name) {
  return EnsureLib() ? reinterpret_cast<Fn>(dlsym(g_lib, name)) : nullptr;
}

napi_value MakeUndefined(napi_env env) {
  napi_value undef = nullptr;
  napi_get_undefined(env, &undef);
  return undef;
}

/// 取第 0 个字符串参数（UTF-8，截断到 4KB：路径不会更长）。
bool GetStringArg(napi_env env, napi_callback_info info, char* out, size_t cap) {
  size_t argc = 1;
  napi_value argv[1] = {nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc < 1) {
    return false;
  }
  size_t len = 0;
  if (napi_get_value_string_utf8(env, argv[0], out, cap, &len) != napi_ok) {
    return false;
  }
  return true;
}

napi_value Init(napi_env env, napi_callback_info info) {
  char dir[4096] = {0};
  if (GetStringArg(env, info, dir, sizeof(dir))) {
    if (auto fn = Sym<InitFn>("yinhe_ohos_init")) {
      fn(dir);
    }
  }
  return MakeUndefined(env);
}

napi_value AddSoundFont(napi_env env, napi_callback_info info) {
  char path[4096] = {0};
  if (GetStringArg(env, info, path, sizeof(path))) {
    if (auto fn = Sym<AddSoundfontFn>("yinhe_ohos_add_soundfont")) {
      fn(path);
    }
  }
  return MakeUndefined(env);
}

napi_value SetSurface(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc >= 1) {
    uint64_t window = 0;
    bool lossless = false;
    if (napi_get_value_bigint_uint64(env, argv[0], &window, &lossless) == napi_ok) {
      if (auto fn = Sym<SetSurfaceFn>("yinhe_ohos_set_surface")) {
        fn(reinterpret_cast<void*>(window));
      }
    }
  }
  return MakeUndefined(env);
}

napi_value ClearSurface(napi_env env, napi_callback_info) {
  if (auto fn = Sym<ClearSurfaceFn>("yinhe_ohos_clear_surface")) {
    fn();
  }
  return MakeUndefined(env);
}

napi_value SetAutomationSurface(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1] = {nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc >= 1) {
    uint64_t window = 0;
    bool lossless = false;
    if (napi_get_value_bigint_uint64(env, argv[0], &window, &lossless) == napi_ok) {
      if (auto fn = Sym<SetSurfaceFn>("yinhe_ohos_set_automation_surface")) {
        fn(reinterpret_cast<void*>(window));
      }
    }
  }
  return MakeUndefined(env);
}

napi_value ClearAutomationSurface(napi_env env, napi_callback_info) {
  if (auto fn = Sym<ClearSurfaceFn>("yinhe_ohos_clear_automation_surface")) {
    fn();
  }
  return MakeUndefined(env);
}

napi_value SetSize(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2] = {nullptr, nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc >= 2) {
    uint32_t width = 1;
    uint32_t height = 1;
    napi_get_value_uint32(env, argv[0], &width);
    napi_get_value_uint32(env, argv[1], &height);
    if (auto fn = Sym<SetSizeFn>("yinhe_ohos_set_size")) {
      fn(width, height);
    }
  }
  return MakeUndefined(env);
}

napi_value SetAutomationSize(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2] = {nullptr, nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc >= 2) {
    uint32_t width = 1;
    uint32_t height = 1;
    napi_get_value_uint32(env, argv[0], &width);
    napi_get_value_uint32(env, argv[1], &height);
    if (auto fn = Sym<SetSizeFn>("yinhe_ohos_set_automation_size")) {
      fn(width, height);
    }
  }
  return MakeUndefined(env);
}

napi_value SetViewport(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2] = {nullptr, nullptr};
  napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
  if (argc >= 2) {
    double width = 1.0;
    double height = 1.0;
    napi_get_value_double(env, argv[0], &width);
    napi_get_value_double(env, argv[1], &height);
    if (auto fn = Sym<SetViewportFn>("yinhe_ohos_set_viewport")) {
      fn(static_cast<float>(width), static_cast<float>(height));
    }
  }
  return MakeUndefined(env);
}

}  // namespace

EXTERN_C_START
static napi_value Register(napi_env env, napi_value exports) {
  napi_property_descriptor desc[] = {
      {"init", nullptr, Init, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"addSoundFont", nullptr, AddSoundFont, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"setSurface", nullptr, SetSurface, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"clearSurface", nullptr, ClearSurface, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"setAutomationSurface", nullptr, SetAutomationSurface, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"clearAutomationSurface", nullptr, ClearAutomationSurface, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"setSize", nullptr, SetSize, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"setAutomationSize", nullptr, SetAutomationSize, nullptr, nullptr, nullptr, napi_default, nullptr},
      {"setViewport", nullptr, SetViewport, nullptr, nullptr, nullptr, napi_default, nullptr},
  };
  napi_define_properties(env, exports, sizeof(desc) / sizeof(desc[0]), desc);
  return exports;
}
EXTERN_C_END

static napi_module g_module = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Register,
    .nm_modname = "yinhe_napi",
    .nm_priv = nullptr,
    .reserved = {0},
};

extern "C" __attribute__((constructor)) void RegisterYinheNapiModule(void) {
  napi_module_register(&g_module);
}
