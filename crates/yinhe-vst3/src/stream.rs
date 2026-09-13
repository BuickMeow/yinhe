//! VST3 host 侧内存流（`IBStream`）：状态保存/恢复与 `setComponentState` 用。
//!
//! 插件通过流读写自己的状态字节；宿主持有缓冲与读写位置。
//! 内部用 `Mutex`（插件理论上只在调用线程读写，但流对象要满足 COM 的线程约束）。

use std::ffi::c_void;
use std::sync::Mutex;

use vst3::Steinberg::IBStream_::IStreamSeekMode_::{kIBSeekCur, kIBSeekEnd, kIBSeekSet};
use vst3::Steinberg::{
    IBStream, IBStreamTrait, int32, int64, kInvalidArgument, kResultOk, tresult,
};
use vst3::{Class, ComWrapper};

/// 可读写内存流。
pub struct MemoryStream {
    inner: Mutex<StreamInner>,
}

struct StreamInner {
    data: Vec<u8>,
    pos: usize,
}

impl Default for MemoryStream {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStream {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(StreamInner {
                data: Vec::new(),
                pos: 0,
            }),
        }
    }

    /// 已写入的数据（状态保存用）。
    pub fn data(&self) -> Vec<u8> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .data
            .clone()
    }

    /// 预置外部数据（状态恢复用），读写位置归零。
    pub fn set_data(&self, bytes: &[u8]) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.data = bytes.to_vec();
        inner.pos = 0;
    }

    /// 读写位置归零（`component.getState` 写完后，`controller.setComponentState` 从头读）。
    pub fn rewind(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).pos = 0;
    }
}

impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_read: *mut int32) -> tresult {
        if buffer.is_null() || num_read.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let available = inner.data.len().saturating_sub(inner.pos);
        let n = (num_bytes as usize).min(available);
        if n > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    inner.data.as_ptr().add(inner.pos),
                    buffer as *mut u8,
                    n,
                );
            }
            inner.pos += n;
        }
        unsafe { *num_read = n as int32 };
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_written: *mut int32,
    ) -> tresult {
        if buffer.is_null() || num_written.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let n = num_bytes as usize;
        let end = inner.pos + n;
        if end > inner.data.len() {
            inner.data.resize(end, 0);
        }
        if n > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buffer as *const u8,
                    inner.data.as_mut_ptr().add(inner.pos),
                    n,
                );
            }
            inner.pos = end;
        }
        unsafe { *num_written = n as int32 };
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let len = inner.data.len() as i64;
        let mode = mode as u32;
        // 常量类型随平台绑定不同（i32/u32），统一转换比较。
        #[allow(clippy::unnecessary_cast)]
        let base = if mode == kIBSeekSet as u32 {
            0
        } else if mode == kIBSeekCur as u32 {
            inner.pos as i64
        } else if mode == kIBSeekEnd as u32 {
            len
        } else {
            return kInvalidArgument;
        };
        let target = (base + pos).clamp(0, len) as usize;
        inner.pos = target;
        if !result.is_null() {
            unsafe { *result = target as i64 };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { *pos = inner.pos as i64 };
        kResultOk
    }
}

/// 创建内存流 COM 对象。
pub(crate) fn create_memory_stream() -> Option<ComWrapper<MemoryStream>> {
    Some(ComWrapper::new(MemoryStream::new()))
}
