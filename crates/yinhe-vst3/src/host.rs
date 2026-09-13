//! VST3 host 侧 COM 对象：`IHostApplication` 与 `IComponentHandler`。
//!
//! 插件在 `initialize(context)` 时拿到宿主应用指针；controller 通过
//! `IComponentHandler` 向宿主回报参数编辑与重启请求。第一版只提供最小实现
//! （参数编辑通知暂不消费，后续接参数面板/自动化时扩展）。

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use vst3::Steinberg::Vst::{
    IAttributeList, IAttributeListTrait, IComponentHandler, IComponentHandlerTrait,
    IHostApplication, IHostApplicationTrait, IMessage, IMessageTrait, ParamID, ParamValue,
    String128,
};
use vst3::Steinberg::{
    FIDString, IPlugFrame, IPlugFrameTrait, IPlugView, TUID, ViewRect, int32, kInvalidArgument,
    kNoInterface, kResultFalse, kResultOk, kResultTrue, tresult,
};
use vst3::{Class, ComPtr, ComWrapper, Interface};

/// 宿主应用（插件的 host context）。
pub struct HostApplication;

impl Class for HostApplication {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApplication {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        let dst = unsafe { &mut *name };
        let mut i = 0;
        for ch in "yinhe".encode_utf16() {
            if i + 1 >= dst.len() {
                break;
            }
            dst[i] = ch;
            i += 1;
        }
        dst[i] = 0;
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        // 提供宿主创建的 IMessage / IAttributeList：分离式 component/controller
        // 通过它们传递消息（部分插件的编辑器在 attached 时依赖，如 Serum 2）。
        if obj.is_null() || cid.is_null() || iid.is_null() {
            return kInvalidArgument;
        }
        unsafe { *obj = ptr::null_mut() };
        let cid_bytes: &[u8; 16] = unsafe { &*(cid as *const [u8; 16]) };
        let iid_bytes: &[u8; 16] = unsafe { &*(iid as *const [u8; 16]) };
        if cid_bytes == &IMessage::IID && iid_bytes == &IMessage::IID {
            if let Some(p) = ComWrapper::new(HostMessage::new()).to_com_ptr::<IMessage>() {
                unsafe { *obj = p.into_raw() as *mut c_void };
                return kResultTrue;
            }
        } else if cid_bytes == &IAttributeList::IID
            && iid_bytes == &IAttributeList::IID
            && let Some(p) =
                ComWrapper::new(HostAttributeList::default()).to_com_ptr::<IAttributeList>()
        {
            unsafe { *obj = p.into_raw() as *mut c_void };
            return kResultTrue;
        }
        kNoInterface
    }
}

/// 属性值（IMessage 负载）。
#[derive(Clone)]
enum AttrValue {
    Int(i64),
    Float(f64),
    /// UTF-16 字符串（不含终止符）。
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

/// 宿主创建的属性列表（`IAttributeList`）。
#[derive(Default)]
pub struct HostAttributeList {
    attrs: Mutex<HashMap<String, AttrValue>>,
}

impl HostAttributeList {
    fn put(&self, key: String, value: AttrValue) {
        self.attrs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, value);
    }

    fn get_value(&self, key: &str) -> Option<AttrValue> {
        self.attrs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }
}

/// `AttrID`（C 字符串）→ 拥有的 key。
unsafe fn attr_key(id: *const std::os::raw::c_char) -> String {
    if id.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned()
}

impl Class for HostAttributeList {
    type Interfaces = (IAttributeList,);
}

impl IAttributeListTrait for HostAttributeList {
    unsafe fn setInt(&self, id: *const std::os::raw::c_char, value: i64) -> tresult {
        self.put(unsafe { attr_key(id) }, AttrValue::Int(value));
        kResultOk
    }

    unsafe fn getInt(&self, id: *const std::os::raw::c_char, value: *mut i64) -> tresult {
        match self.get_value(&unsafe { attr_key(id) }) {
            Some(AttrValue::Int(v)) if !value.is_null() => {
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: *const std::os::raw::c_char, value: f64) -> tresult {
        self.put(unsafe { attr_key(id) }, AttrValue::Float(value));
        kResultOk
    }

    unsafe fn getFloat(&self, id: *const std::os::raw::c_char, value: *mut f64) -> tresult {
        match self.get_value(&unsafe { attr_key(id) }) {
            Some(AttrValue::Float(v)) if !value.is_null() => {
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: *const std::os::raw::c_char, string: *const u16) -> tresult {
        if string.is_null() {
            return kResultFalse;
        }
        let mut buf = Vec::new();
        let mut p = string;
        unsafe {
            while *p != 0 {
                buf.push(*p);
                p = p.add(1);
            }
        }
        self.put(unsafe { attr_key(id) }, AttrValue::Str(buf));
        kResultOk
    }

    unsafe fn getString(
        &self,
        id: *const std::os::raw::c_char,
        string: *mut u16,
        size_in_bytes: u32,
    ) -> tresult {
        match self.get_value(&unsafe { attr_key(id) }) {
            Some(AttrValue::Str(v)) if !string.is_null() => {
                let cap_chars = (size_in_bytes as usize / 2).saturating_sub(1);
                let n = v.len().min(cap_chars);
                unsafe {
                    for (i, &ch) in v.iter().take(n).enumerate() {
                        *string.add(i) = ch;
                    }
                    *string.add(n) = 0;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setBinary(
        &self,
        id: *const std::os::raw::c_char,
        data: *const c_void,
        size_in_bytes: u32,
    ) -> tresult {
        if data.is_null() {
            return kResultFalse;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }
                .to_vec();
        self.put(unsafe { attr_key(id) }, AttrValue::Bin(bytes));
        kResultOk
    }

    unsafe fn getBinary(
        &self,
        id: *const std::os::raw::c_char,
        data: *mut *const c_void,
        size_in_bytes: *mut u32,
    ) -> tresult {
        if data.is_null() || size_in_bytes.is_null() {
            return kResultFalse;
        }
        let attrs = self.attrs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(AttrValue::Bin(v)) = attrs.get(&unsafe { attr_key(id) }) {
            unsafe {
                *data = v.as_ptr() as *const c_void;
                *size_in_bytes = v.len() as u32;
            }
            return kResultOk;
        }
        kResultFalse
    }
}

/// 宿主创建的消息（`IMessage`：id + 属性列表）。
pub struct HostMessage {
    id: Mutex<Option<CString>>,
    attributes: ComWrapper<HostAttributeList>,
}

impl Default for HostMessage {
    fn default() -> Self {
        Self {
            id: Mutex::new(None),
            attributes: ComWrapper::new(HostAttributeList::default()),
        }
    }
}

impl HostMessage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        let guard = self.id.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(s) => s.as_ptr(),
            None => ptr::null(),
        }
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        if id.is_null() {
            return;
        }
        let owned = unsafe { CStr::from_ptr(id) }.to_owned();
        *self.id.lock().unwrap_or_else(|e| e.into_inner()) = Some(owned);
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attributes
            .to_com_ptr::<IAttributeList>()
            .map(|p| p.as_ptr())
            .unwrap_or(ptr::null_mut())
    }
}

/// 组件处理器（controller → host 通知）。
#[derive(Default)]
pub struct HostComponentHandler;

impl Class for HostComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for HostComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, _id: ParamID, _value_normalized: ParamValue) -> tresult {
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        kResultOk
    }
}

/// 创建 host 应用对象并返回 COM 指针（与组件同生命周期）。
pub(crate) fn create_host_application() -> Option<ComPtr<IHostApplication>> {
    ComWrapper::new(HostApplication).to_com_ptr::<IHostApplication>()
}

/// 创建组件处理器对象并返回 COM 指针。
pub(crate) fn create_component_handler() -> Option<ComPtr<IComponentHandler>> {
    ComWrapper::new(HostComponentHandler).to_com_ptr::<IComponentHandler>()
}

/// 编辑器 frame（host 侧）：插件请求调整窗口尺寸的中转。
///
/// 插件在自己的线程调用 `resizeView`，这里只记原子请求；
/// UI 轮询后调整宿主 NSWindow 并回调 `IPlugView::onSize`。
#[derive(Default)]
pub struct HostPlugFrame {
    /// packed `(w << 32) | h`；0 = 无请求。
    resize_request: AtomicU64,
}

impl HostPlugFrame {
    /// 取出尺寸调整请求（取出即清除）。
    pub fn take_resize(&self) -> Option<(u32, u32)> {
        let raw = self.resize_request.swap(0, Ordering::AcqRel);
        if raw == 0 {
            return None;
        }
        Some(((raw >> 32) as u32, raw as u32))
    }
}

impl Class for HostPlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for HostPlugFrame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        if new_size.is_null() {
            return kInvalidArgument;
        }
        let rect = unsafe { &*new_size };
        let w = (rect.right - rect.left).max(0) as u64;
        let h = (rect.bottom - rect.top).max(0) as u64;
        self.resize_request.store((w << 32) | h, Ordering::Release);
        kResultOk
    }
}

/// 创建编辑器 frame 对象。
pub(crate) fn create_plug_frame() -> Option<ComWrapper<HostPlugFrame>> {
    Some(ComWrapper::new(HostPlugFrame::default()))
}
