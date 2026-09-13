//! VST3 插件实例（管理线程侧）：创建 component/controller、参数枚举与读写。
//!
//! 两段式（与 yinhe-clap 对齐）：
//! - 本类型在管理/UI 线程持有 IComponent/IAudioProcessor 与 IEditController；
//! - 后续阶段从本类型产出 `Send` 的音频处理器（move 进渲染线程）。
//!
//! 线程约定：所有方法都在加载插件的线程调用（第一版=UI 主线程）。

use std::path::Path;
use std::ptr;

use vst3::Steinberg::Vst::{
    IComponent, IComponentHandler, IComponentTrait, IConnectionPoint, IConnectionPointTrait,
    IEditController, IEditControllerTrait, IHostApplication, ParameterInfo, String128,
};
use vst3::Steinberg::{
    FUnknown, IBStream, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait, TUID, kResultOk,
};
use vst3::{ComPtr, ComRef, Interface};

use crate::factory::parse_uid;
use crate::host::{create_component_handler, create_host_application};
use crate::loader::{LoadError, LoadedModule};

/// 实例创建失败。
#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("加载模块失败: {0}")]
    Load(#[from] LoadError),
    #[error("非法 class id: {0}")]
    BadClassId(String),
    #[error("factory 创建对象失败（class id {0}）")]
    CreateFailed(String),
    #[error("初始化失败: {0}")]
    Initialize(String),
    #[error("状态错误: {0}")]
    State(String),
}

/// 插件参数描述（`ParameterInfo` 子集）。
#[derive(Clone, Debug, PartialEq)]
pub struct Vst3ParamInfo {
    pub id: u32,
    pub title: String,
    pub units: String,
    pub default_normalized: f64,
    pub step_count: i32,
    pub flags: i32,
}

/// 一个已创建（未激活音频）的 VST3 插件实例。
pub struct Vst3PluginInstance {
    // ── drop 顺序（字段正序）：component → controller → cps → handler → host → module ──
    component: ComPtr<IComponent>,
    controller: ComPtr<IEditController>,
    component_cp: Option<ComPtr<IConnectionPoint>>,
    controller_cp: Option<ComPtr<IConnectionPoint>>,
    _handler: ComPtr<IComponentHandler>,
    _host: ComPtr<IHostApplication>,
    _module: LoadedModule,
    /// 分离式 controller（与 component 不是同一 COM 对象；terminate 要分别调用）。
    separate_controller: bool,
    params: Vec<Vst3ParamInfo>,
    class_id: String,
}

impl Vst3PluginInstance {
    /// 加载模块并创建/初始化组件与控制器。
    pub fn load(bundle: &Path, class_id: &str) -> Result<Self, InstanceError> {
        let module = LoadedModule::load(bundle)?;
        let factory = unsafe { ComRef::from_raw(module.factory_ptr()) }
            .ok_or_else(|| InstanceError::CreateFailed(class_id.to_string()))?;
        let cid = parse_uid(class_id).ok_or_else(|| InstanceError::BadClassId(class_id.into()))?;

        // host context（生命周期覆盖组件：字段声明在组件之后）。
        let host = create_host_application()
            .ok_or_else(|| InstanceError::Initialize("无法创建 host 应用对象".into()))?;
        let context = host.as_ptr() as *mut FUnknown;

        // component。
        let component = create_instance::<IComponent>(factory, &cid)
            .ok_or_else(|| InstanceError::CreateFailed(class_id.to_string()))?;

        // controller：单组件（同一对象实现 IEditController）优先，否则独立类。
        let (controller, separate_controller) = match component.cast::<IEditController>() {
            Some(c) => (c, false),
            None => {
                let mut controller_cid: TUID = [0; 16];
                let r = unsafe { component.getControllerClassId(&mut controller_cid) };
                if r != kResultOk {
                    return Err(InstanceError::Initialize(format!(
                        "组件未提供 controller（getControllerClassId={r:#x}）"
                    )));
                }
                let c = create_instance::<IEditController>(factory, &controller_cid)
                    .ok_or_else(|| InstanceError::CreateFailed(class_id.to_string()))?;
                (c, true)
            }
        };

        // initialize：分离式 controller 也要初始化。
        let r = unsafe { component.initialize(context) };
        if r != kResultOk {
            return Err(InstanceError::Initialize(format!(
                "component.initialize 失败（{r:#x}）"
            )));
        }
        if separate_controller {
            let r = unsafe { controller.initialize(context) };
            if r != kResultOk {
                let _ = unsafe { component.terminate() };
                return Err(InstanceError::Initialize(format!(
                    "controller.initialize 失败（{r:#x}）"
                )));
            }
        }

        // 分离式：连接 component ↔ controller（消息/参数通道）。
        let (component_cp, controller_cp) = if separate_controller {
            let ccp = component.cast::<IConnectionPoint>();
            let kcp = controller.cast::<IConnectionPoint>();
            if let (Some(ccp), Some(kcp)) = (&ccp, &kcp) {
                let _ = unsafe { ccp.connect(kcp.as_ptr()) };
            }
            (ccp, kcp)
        } else {
            (None, None)
        };

        // 分离式：把组件状态同步给 controller（部分插件在此才装载参数列表）。
        if separate_controller {
            sync_component_state(&component, &controller);
        }

        // 组件处理器：controller 的参数编辑/重启通知入口。
        let handler = create_component_handler()
            .ok_or_else(|| InstanceError::Initialize("无法创建组件处理器对象".into()))?;
        let _ = unsafe { controller.setComponentHandler(handler.as_ptr()) };

        let params = enumerate_params(&controller);

        Ok(Self {
            component,
            controller,
            component_cp,
            controller_cp,
            _handler: handler,
            _host: host,
            _module: module,
            separate_controller,
            params,
            class_id: class_id.to_string(),
        })
    }

    pub fn class_id(&self) -> &str {
        &self.class_id
    }

    /// 参数列表（创建时枚举一次；插件 rescan 后需重建实例/重新枚举）。
    pub fn params(&self) -> &[Vst3ParamInfo] {
        &self.params
    }

    /// 读取参数当前归一化值（0..1）。
    pub fn get_param_normalized(&self, id: u32) -> f64 {
        unsafe { self.controller.getParamNormalized(id) }
    }

    /// 写入参数值（controller 侧立即生效；component 侧由后续音频事件同步）。
    pub fn set_param_normalized(&self, id: u32, value: f64) {
        unsafe {
            let _ = self.controller.setParamNormalized(id, value);
        }
    }

    /// 参数的插件侧格式化文本（如 "8.2 kHz"）。
    pub fn format_param(&self, id: u32, value: f64) -> Option<String> {
        let mut text: String128 = [0; 128];
        let r = unsafe { self.controller.getParamStringByValue(id, value, &mut text) };
        (r == kResultOk).then(|| utf16_str(&text))
    }

    /// 保存状态（component + controller 两段，合并序列化）。
    ///
    /// 格式：`[u32 LE component_len][component][u32 LE controller_len][controller]`。
    /// 段为空（插件 not supported / 返回失败）时该段长度为 0。
    ///
    /// **调用时机**：必须在音频激活（setupProcessing/setActive）之后。实测部分
    /// 插件（Serum 2）在未激活时 `getState` 会段错误（进程内崩溃）。
    pub fn save_state(&self) -> Vec<u8> {
        let component_bytes = self.get_component_state().unwrap_or_default();
        let controller_bytes = self.get_controller_state().unwrap_or_default();
        let mut out = Vec::with_capacity(8 + component_bytes.len() + controller_bytes.len());
        out.extend_from_slice(&(component_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&component_bytes);
        out.extend_from_slice(&(controller_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&controller_bytes);
        out
    }

    /// 恢复状态（先同步 component → controller，再恢复 controller 自身状态）。
    pub fn load_state(&self, bytes: &[u8]) -> Result<(), InstanceError> {
        let (component_bytes, controller_bytes) =
            split_state(bytes).ok_or_else(|| InstanceError::State("状态字节格式非法".into()))?;
        if !component_bytes.is_empty() {
            let stream = crate::stream::create_memory_stream()
                .ok_or_else(|| InstanceError::State("无法创建内存流".into()))?;
            stream.set_data(component_bytes);
            let ptr = stream
                .to_com_ptr::<IBStream>()
                .ok_or_else(|| InstanceError::State("内存流接口查询失败".into()))?;
            unsafe {
                let _ = self.controller.setComponentState(ptr.as_ptr());
            }
            stream.rewind();
            unsafe {
                let _ = self.component.setState(ptr.as_ptr());
            }
        }
        if !controller_bytes.is_empty() {
            let stream = crate::stream::create_memory_stream()
                .ok_or_else(|| InstanceError::State("无法创建内存流".into()))?;
            stream.set_data(controller_bytes);
            let ptr = stream
                .to_com_ptr::<IBStream>()
                .ok_or_else(|| InstanceError::State("内存流接口查询失败".into()))?;
            unsafe {
                let _ = self.controller.setState(ptr.as_ptr());
            }
        }
        Ok(())
    }

    fn get_component_state(&self) -> Result<Vec<u8>, InstanceError> {
        let stream = crate::stream::create_memory_stream()
            .ok_or_else(|| InstanceError::State("无法创建内存流".into()))?;
        let ptr = stream
            .to_com_ptr::<IBStream>()
            .ok_or_else(|| InstanceError::State("内存流接口查询失败".into()))?;
        let r = unsafe { self.component.getState(ptr.as_ptr()) };
        if r != kResultOk {
            return Err(InstanceError::State(format!("component.getState={r:#x}")));
        }
        Ok(stream.data())
    }

    fn get_controller_state(&self) -> Result<Vec<u8>, InstanceError> {
        let stream = crate::stream::create_memory_stream()
            .ok_or_else(|| InstanceError::State("无法创建内存流".into()))?;
        let ptr = stream
            .to_com_ptr::<IBStream>()
            .ok_or_else(|| InstanceError::State("内存流接口查询失败".into()))?;
        let r = unsafe { self.controller.getState(ptr.as_ptr()) };
        if r != kResultOk {
            return Err(InstanceError::State(format!("controller.getState={r:#x}")));
        }
        Ok(stream.data())
    }
}

/// 解析 [`Vst3PluginInstance::save_state`] 的两段格式。
fn split_state(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let component_len = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    let component = bytes.get(4..4 + component_len)?;
    let rest = &bytes[4 + component_len..];
    let controller_len = u32::from_le_bytes(rest.get(0..4)?.try_into().ok()?) as usize;
    let controller = rest.get(4..4 + controller_len)?;
    Some((component, controller))
}

impl Drop for Vst3PluginInstance {
    fn drop(&mut self) {
        unsafe {
            if let (Some(ccp), Some(kcp)) = (&self.component_cp, &self.controller_cp) {
                let _ = ccp.disconnect(kcp.as_ptr());
            }
            if self.separate_controller {
                let _ = self.controller.terminate();
            }
            let _ = self.component.terminate();
        }
    }
}

/// 把 component 的状态同步给 controller（分离式插件参数在此时装载）。
fn sync_component_state(component: &ComPtr<IComponent>, controller: &ComPtr<IEditController>) {
    let Some(stream) = crate::stream::create_memory_stream() else {
        return;
    };
    let Some(stream_ptr) = stream.to_com_ptr::<IBStream>() else {
        return;
    };
    unsafe {
        let r = component.getState(stream_ptr.as_ptr());
        if r != kResultOk {
            tracing::warn!("component.getState 失败（{r:#x}）");
            return;
        }
    }
    stream.rewind();
    unsafe {
        let r = controller.setComponentState(stream_ptr.as_ptr());
        if r != kResultOk {
            tracing::warn!("controller.setComponentState 失败（{r:#x}）");
        }
    }
}

/// 用 factory 创建指定接口的 COM 对象。
fn create_instance<T: Interface>(factory: ComRef<IPluginFactory>, cid: &TUID) -> Option<ComPtr<T>> {
    let mut obj: *mut std::ffi::c_void = ptr::null_mut();
    let r = unsafe {
        factory.createInstance(
            cid.as_ptr(),
            T::IID.as_ptr() as *const std::ffi::c_char,
            &mut obj,
        )
    };
    if r != kResultOk || obj.is_null() {
        return None;
    }
    unsafe { ComPtr::from_raw(obj as *mut T) }
}

/// 枚举 controller 参数。
fn enumerate_params(controller: &ComPtr<IEditController>) -> Vec<Vst3ParamInfo> {
    unsafe {
        let count = controller.getParameterCount();
        (0..count)
            .filter_map(|i| {
                let mut info: ParameterInfo = std::mem::zeroed();
                if controller.getParameterInfo(i, &mut info) != kResultOk {
                    return None;
                }
                Some(Vst3ParamInfo {
                    id: info.id,
                    title: utf16_str(&info.title),
                    units: utf16_str(&info.units),
                    default_normalized: info.defaultNormalizedValue,
                    step_count: info.stepCount,
                    flags: info.flags,
                })
            })
            .collect()
    }
}

/// UTF-16 定长缓冲 → String（截断到首个 NUL）。
fn utf16_str(raw: &[u16]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}
