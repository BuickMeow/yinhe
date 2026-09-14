//! VST3 插件实例（管理线程侧）：创建 component/controller、参数枚举与读写。
//!
//! 两段式（与 yinhe-clap 对齐）：
//! - 本类型在管理/UI 线程持有 IComponent/IAudioProcessor 与 IEditController；
//! - 后续阶段从本类型产出 `Send` 的音频处理器（move 进渲染线程）。
//!
//! 线程约定：所有方法都在加载插件的线程调用（第一版=UI 主线程）。

use std::path::Path;
use std::ptr;

use std::sync::Arc;

use vst3::Steinberg::Vst::{
    BusDirections_::{kInput, kOutput},
    IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait,
    IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
    IHostApplication,
    MediaTypes_::kAudio,
    ParameterInfo,
    ProcessModes_::kRealtime,
    ProcessSetup, String128,
    SymbolicSampleSizes_::kSample32,
};
use vst3::Steinberg::{
    FUnknown, IBStream, IPlugFrame, IPlugView, IPlugViewContentScaleSupport,
    IPlugViewContentScaleSupportTrait, IPlugViewTrait, IPluginBaseTrait, IPluginFactory,
    IPluginFactoryTrait, TUID, ViewRect, kNotImplemented, kPlatformTypeNSView, kResultOk,
};
use vst3::{ComPtr, ComRef, ComWrapper, Interface};

use yinhe_mixer::ParamQueue;

use crate::factory::parse_uid;
use crate::host::{create_component_handler, create_host_application};
use crate::loader::{LoadError, LoadedModule};
use crate::processor::Vst3Processor;

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
    /// controller 的 restartComponent flags（共享原子；管理线程轮询消费）。
    restart_flags: std::sync::Arc<std::sync::atomic::AtomicI32>,
    _host: ComPtr<IHostApplication>,
    _module: LoadedModule,
    /// 分离式 controller（与 component 不是同一 COM 对象；terminate 要分别调用）。
    separate_controller: bool,
    params: Vec<Vst3ParamInfo>,
    class_id: String,
    /// UI → 渲染线程的参数变化队列。
    param_queue: Arc<ParamQueue>,
    /// 插件 GUI 改参队列（performEdit，归一化值）→ 宿主（自动化录制）。
    gui_params: Arc<ParamQueue>,
    /// 是否处于 beginEdit/endEdit 之间（一次拖动 = 一条 undo）。
    gui_editing: Arc<std::sync::atomic::AtomicBool>,
    /// 是否已音频激活（未激活时 getState 有崩溃风险——实测 Serum 2）。
    activated: std::sync::atomic::AtomicBool,
    // ── 编辑器（原生 GUI）──
    /// 插件编辑器视图（createView 产出，drop 前必须 close_view）。
    view: Option<ComPtr<IPlugView>>,
    /// host 侧 frame（持有引用计数）。
    plug_frame: Option<ComWrapper<crate::host::HostPlugFrame>>,
    /// frame 的接口指针（setFrame 用）。
    frame_ptr: Option<ComPtr<IPlugFrame>>,
    /// 是否已嵌入宿主 view（attached 状态）。
    gui_attached: bool,
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
        let _ = unsafe { controller.setComponentHandler(handler.ptr.as_ptr()) };
        let restart_flags = Arc::clone(&handler.restart_flags);
        let gui_params = Arc::clone(&handler.gui_params);
        let gui_editing = Arc::clone(&handler.editing);

        let params = enumerate_params(&controller);

        Ok(Self {
            component,
            controller,
            component_cp,
            controller_cp,
            _handler: handler.ptr,
            restart_flags,
            _host: host,
            _module: module,
            separate_controller,
            params,
            class_id: class_id.to_string(),
            param_queue: Arc::new(ParamQueue::new()),
            gui_params,
            gui_editing,
            activated: std::sync::atomic::AtomicBool::new(false),
            view: None,
            plug_frame: None,
            frame_ptr: None,
            gui_attached: false,
        })
    }

    pub fn class_id(&self) -> &str {
        &self.class_id
    }

    /// 参数写入队列（UI 线程 push；处理器在渲染线程 drain）。
    pub fn param_queue(&self) -> Arc<ParamQueue> {
        Arc::clone(&self.param_queue)
    }

    /// 取出插件 GUI 改参（performEdit，归一化值）与当前是否处于编辑拖动中。
    /// 宿主每帧轮询；队列 latest-wins、取出即清。
    pub fn take_gui_param_changes(&self) -> (Vec<(u32, f64)>, bool) {
        let mut out = Vec::new();
        self.gui_params.take_into(&mut out);
        let editing = self.gui_editing.load(std::sync::atomic::Ordering::Acquire);
        (out, editing)
    }

    /// 是否已音频激活（激活前 getState 有崩溃风险，保存状态前必须检查）。
    pub fn is_activated(&self) -> bool {
        self.activated.load(std::sync::atomic::Ordering::Acquire)
    }

    // ── 原生 GUI（macOS：NSView 嵌入；与 CLAP 共用宿主 NSWindow）──

    /// 创建编辑器视图（幂等）并返回首选尺寸。
    pub fn create_view(&mut self) -> Result<(u32, u32), InstanceError> {
        if self.view.is_none() {
            // 先用 "editor"；部分插件只提供默认视图（空名）。
            let mut view_ptr = unsafe { self.controller.createView(c"editor".as_ptr()) };
            if view_ptr.is_null() {
                view_ptr = unsafe { self.controller.createView(c"".as_ptr()) };
            }
            let Some(view) = (unsafe { ComPtr::from_raw(view_ptr) }) else {
                return Err(InstanceError::Initialize(
                    "插件没有编辑器（createView 返回空）".into(),
                ));
            };
            let r = unsafe { view.isPlatformTypeSupported(kPlatformTypeNSView) };
            if r != kResultOk {
                return Err(InstanceError::Initialize(
                    "插件编辑器不支持 NSView 嵌入".into(),
                ));
            }
            let frame = crate::host::create_plug_frame()
                .ok_or_else(|| InstanceError::Initialize("创建 plug frame 失败".into()))?;
            let frame_ptr = frame
                .to_com_ptr::<IPlugFrame>()
                .ok_or_else(|| InstanceError::Initialize("plug frame 接口查询失败".into()))?;
            unsafe {
                let _ = view.setFrame(frame_ptr.as_ptr());
            }
            // Retina 缩放：Serum 等插件在 attached 前依赖已设置的 scale factor
            //（接口可选；不实现则忽略）。
            if let Some(scale) = view.cast::<IPlugViewContentScaleSupport>() {
                let _ = unsafe { scale.setContentScaleFactor(2.0) };
            }
            self.view = Some(view);
            self.plug_frame = Some(frame);
            self.frame_ptr = Some(frame_ptr);
        }
        let Some(view) = &self.view else {
            return Err(InstanceError::Initialize("编辑器创建失败".into()));
        };
        let mut rect: ViewRect = unsafe { std::mem::zeroed() };
        let r = unsafe { view.getSize(&mut rect) };
        if r != kResultOk {
            return Err(InstanceError::Initialize(format!(
                "获取编辑器尺寸失败（{r:#x}）"
            )));
        }
        Ok((
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        ))
    }

    /// 把编辑器嵌入宿主 view（macOS: NSView 指针）。
    ///
    /// # Safety
    /// `parent` 必须是有效的、生命周期覆盖编辑器关闭时机的平台窗口句柄
    /// （macOS NSView；由调用方保证存活）。
    pub unsafe fn attach_view(
        &mut self,
        parent: *mut std::ffi::c_void,
    ) -> Result<(), InstanceError> {
        let Some(view) = &self.view else {
            return Err(InstanceError::Initialize("编辑器未创建".into()));
        };
        let r = unsafe { view.attached(parent, kPlatformTypeNSView) };
        if r != kResultOk {
            return Err(InstanceError::Initialize(format!(
                "编辑器嵌入失败（{r:#x}）"
            )));
        }
        self.gui_attached = true;
        Ok(())
    }

    /// 关闭并销毁编辑器（重复调用幂等）。
    pub fn close_view(&mut self) {
        if let Some(view) = self.view.take() {
            unsafe {
                if self.gui_attached {
                    let _ = view.removed();
                    let _ = view.attached(std::ptr::null_mut(), kPlatformTypeNSView);
                }
                let _ = view.setFrame(std::ptr::null_mut());
            }
            self.gui_attached = false;
        }
        self.frame_ptr = None;
        self.plug_frame = None;
    }

    /// 插件请求的窗口尺寸（取出即清除）。
    pub fn take_view_resize(&self) -> Option<(u32, u32)> {
        self.plug_frame.as_ref().and_then(|f| f.take_resize())
    }

    /// 宿主窗口调整后通知插件（VST3 规范要求）。
    pub fn notify_view_resize(&self, width: u32, height: u32) {
        let Some(view) = &self.view else {
            return;
        };
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        unsafe {
            let _ = view.onSize(&mut rect);
        }
    }

    /// 激活音频并产出渲染线程处理器。
    ///
    /// 流程：`setupProcessing` → 激活主音频总线（其余不激活）→ `setActive(true)`
    /// → `setProcessing(true)`。管理线程调用。
    pub fn activate_audio(
        &self,
        sample_rate: f64,
        max_frames: u32,
    ) -> Result<Vst3Processor, InstanceError> {
        let processor = self
            .component
            .cast::<IAudioProcessor>()
            .ok_or_else(|| InstanceError::Initialize("组件不支持 IAudioProcessor".into()))?;

        let mut setup = ProcessSetup {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            maxSamplesPerBlock: max_frames as i32,
            sampleRate: sample_rate,
        };
        let r = unsafe { processor.setupProcessing(&mut setup) };
        if r != kResultOk {
            return Err(InstanceError::Initialize(format!(
                "setupProcessing 失败（{r:#x}）"
            )));
        }

        let n_in = unsafe { self.component.getBusCount(kAudio as i32, kInput as i32) };
        let n_out = unsafe { self.component.getBusCount(kAudio as i32, kOutput as i32) };
        if n_out <= 0 {
            return Err(InstanceError::Initialize("插件没有音频输出总线".into()));
        }
        unsafe {
            if n_in > 0 {
                let _ = self
                    .component
                    .activateBus(kAudio as i32, kInput as i32, 0, 1);
            }
            let _ = self
                .component
                .activateBus(kAudio as i32, kOutput as i32, 0, 1);
        }

        let r = unsafe { self.component.setActive(1) };
        if r != kResultOk {
            return Err(InstanceError::Initialize(format!(
                "setActive 失败（{r:#x}）"
            )));
        }
        // `setProcessing` 是可选通知：插件可以返回 kNotImplemented（新旧 SDK
        // 常量值分别为 0x80004001 / 3，如 kHs 系列），不算错误。
        const K_NOT_IMPLEMENTED_OLD: i32 = 3;
        let r = unsafe { processor.setProcessing(1) };
        if r != kResultOk && r != kNotImplemented && r != K_NOT_IMPLEMENTED_OLD {
            let _ = unsafe { self.component.setActive(0) };
            return Err(InstanceError::Initialize(format!(
                "setProcessing 失败（{r:#x}）"
            )));
        }

        let processor = Vst3Processor::new(
            self.component.clone(),
            processor,
            sample_rate,
            max_frames as usize,
            Arc::clone(&self.param_queue),
        )
        .ok_or_else(|| InstanceError::Initialize("音频缓冲/宿主对象构造失败".into()))?;
        self.activated
            .store(true, std::sync::atomic::Ordering::Release);
        // 激活后组件状态才可读（Serum 等未激活 getState 失败）：重新同步给 controller，
        // 编辑器与参数显示依赖它。
        sync_component_state(&self.component, &self.controller);
        Ok(processor)
    }

    /// 参数列表（创建时枚举；插件请求重扫后由 [`refresh_params`](Self::refresh_params) 更新）。
    pub fn params(&self) -> &[Vst3ParamInfo] {
        &self.params
    }

    /// 取出并清除累积的 restartComponent flags（0 = 无请求）。
    pub fn take_restart_flags(&self) -> i32 {
        self.restart_flags
            .swap(0, std::sync::atomic::Ordering::AcqRel)
    }

    /// 重新枚举参数（插件请求参数值/标题变化后调用）。
    pub fn refresh_params(&mut self) {
        self.params = enumerate_params(&self.controller);
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
        // GUI 必须先于 controller/component 释放（插件 view 引用控制器/组件）。
        self.close_view();
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
            // 未激活时部分插件（Element/Serum）getState 返回失败：正常现象，
            // 参数列表通常不依赖它；激活后如需状态同步由 save_state 处理。
            tracing::debug!("component.getState 失败（{r:#x}，未激活时常见）");
            return;
        }
    }
    stream.rewind();
    unsafe {
        let r = controller.setComponentState(stream_ptr.as_ptr());
        if r != kResultOk {
            tracing::debug!("controller.setComponentState 失败（{r:#x}）");
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
