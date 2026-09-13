//! host 提供给插件的事件列表与参数变化队列（VST3 COM 对象）。
//!
//! process 期间宿主把输入事件/参数变化以这些对象传给插件。
//! 输出侧第一版只保留对象（插件可写），暂不消费。

use std::ptr;
use std::sync::Mutex;

use vst3::Steinberg::Vst::{
    Event, IEventList, IEventListTrait, IParamValueQueue, IParamValueQueueTrait, IParameterChanges,
    IParameterChangesTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

/// 事件列表（`IEventList`）：宿主填充输入事件，插件经 `getEvent` 读取。
#[derive(Default)]
pub struct HostEventList {
    events: Mutex<Vec<Event>>,
}

impl HostEventList {
    pub fn new() -> Self {
        Self::default()
    }

    /// 宿主侧：清空（每次 process 前）。
    pub fn clear(&self) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// 宿主侧：追加事件。
    pub fn push(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
    }

    /// 宿主侧：取出（并清空）插件写入的输出事件。
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Class for HostEventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for HostEventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        if e.is_null() || index < 0 {
            return kInvalidArgument;
        }
        let events = self.events.lock().unwrap_or_else(|err| err.into_inner());
        let Some(event) = events.get(index as usize) else {
            return kInvalidArgument;
        };
        unsafe { *e = *event };
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        self.events
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(unsafe { *e });
        kResultOk
    }
}

/// 单参数的自动化点队列（`IParamValueQueue`）。
pub struct ParamValueQueue {
    id: ParamID,
    points: Mutex<Vec<(int32, ParamValue)>>,
}

impl ParamValueQueue {
    pub fn new(id: ParamID) -> Self {
        Self {
            id,
            points: Mutex::new(Vec::new()),
        }
    }

    pub fn id(&self) -> ParamID {
        self.id
    }

    /// 宿主侧：清空点（队列对象保留复用）。
    pub fn clear(&self) {
        self.points
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// 宿主侧：追加一个点。
    pub fn push(&self, sample_offset: int32, value: ParamValue) {
        self.points
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((sample_offset, value));
    }

    fn has_points(&self) -> bool {
        !self
            .points
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

impl Class for ParamValueQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamValueQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.lock().unwrap_or_else(|e| e.into_inner()).len() as int32
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        sample_offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        if sample_offset.is_null() || value.is_null() || index < 0 {
            return kInvalidArgument;
        }
        let points = self.points.lock().unwrap_or_else(|e| e.into_inner());
        let Some((offset, v)) = points.get(index as usize) else {
            return kInvalidArgument;
        };
        unsafe {
            *sample_offset = *offset;
            *value = *v;
        }
        kResultOk
    }

    unsafe fn addPoint(
        &self,
        sample_offset: int32,
        value: ParamValue,
        index: *mut int32,
    ) -> tresult {
        let mut points = self.points.lock().unwrap_or_else(|e| e.into_inner());
        let i = points.len() as int32;
        points.push((sample_offset, value));
        if !index.is_null() {
            unsafe { *index = i };
        }
        kResultOk
    }
}

/// 参数变化集合（`IParameterChanges`）：每个变化参数一个 [`ParamValueQueue`]。
#[derive(Default)]
pub struct HostParamChanges {
    queues: Mutex<Vec<QueueEntry>>,
}

struct QueueEntry {
    wrapper: ComWrapper<ParamValueQueue>,
    ptr: ComPtr<IParamValueQueue>,
}

impl HostParamChanges {
    pub fn new() -> Self {
        Self::default()
    }

    /// 宿主侧：清空全部点（队列对象保留复用）。
    pub fn clear(&self) {
        for entry in self.queues.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            entry.wrapper.clear();
        }
    }

    /// 宿主侧：为参数追加一个变化点（队列懒创建）。
    pub fn push_point(&self, id: ParamID, sample_offset: int32, value: ParamValue) {
        let mut queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = queues.iter().find(|e| e.wrapper.id() == id) {
            entry.wrapper.push(sample_offset, value);
            return;
        }
        let wrapper = ComWrapper::new(ParamValueQueue::new(id));
        let Some(ptr) = wrapper.to_com_ptr::<IParamValueQueue>() else {
            return;
        };
        wrapper.push(sample_offset, value);
        queues.push(QueueEntry { wrapper, ptr });
    }
}

impl Class for HostParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for HostParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.queues
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|e| e.wrapper.has_points())
            .count() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 {
            return ptr::null_mut();
        }
        let queues = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        // 只暴露有点的队列（空队列对插件无意义）。
        queues
            .iter()
            .filter(|e| e.wrapper.has_points())
            .nth(index as usize)
            .map(|e| e.ptr.as_ptr())
            .unwrap_or(ptr::null_mut())
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        // 第一版不消费插件的输出参数变化。
        if !index.is_null() {
            unsafe { *index = -1 };
        }
        ptr::null_mut()
    }
}
