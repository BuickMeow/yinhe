//! VST3 host 侧 COM 对象：`IHostApplication` 与 `IComponentHandler`。
//!
//! 插件在 `initialize(context)` 时拿到宿主应用指针；controller 通过
//! `IComponentHandler` 向宿主回报参数编辑与重启请求。第一版只提供最小实现
//! （参数编辑通知暂不消费，后续接参数面板/自动化时扩展）。

use std::ffi::c_void;
use std::ptr;

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, IHostApplication, IHostApplicationTrait, ParamID,
    ParamValue, String128,
};
use vst3::Steinberg::{TUID, int32, kInvalidArgument, kNoInterface, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

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
        _cid: *mut TUID,
        _iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        // 第一版不提供 host 创建的辅助对象（IMessage/IAttributeList 等）。
        if !obj.is_null() {
            unsafe { *obj = ptr::null_mut() };
        }
        kNoInterface
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
