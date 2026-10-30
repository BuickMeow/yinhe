cmake_minimum_required(VERSION 3.5.0)
project(yinhe_napi)

set(NATIVERENDER_ROOT_PATH ${CMAKE_CURRENT_SOURCE_DIR})

include_directories(${NATIVERENDER_ROOT_PATH})

add_library(yinhe_napi SHARED yinhe_napi.cpp)

# libace_napi 提供 NAPI 运行时；libhilog 仅用于日志（暂未使用可去掉）。
target_link_libraries(yinhe_napi PUBLIC libace_napi.z.so libhilog_ndk.z.so)
