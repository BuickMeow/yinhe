//! 事件编辑原语（声明宏），收敛 `conductor_edit` / `track_meta_edit` 中
//! 逐字重复的 set / delete / insert 模板。
//!
//! 约定（调用方须已 `use std::sync::Arc`）：
//! - `$self` 传方法接收者 `self`，`$field` 传事件列表字段名；
//! - 事件类型须有 `tick: u32` 字段，编辑后按 tick 稳定排序；
//! - 前缀 `rebuild tempo_map;` 表示编辑后重建 TempoMap（time_sig 专用）；
//! - 未命中时 conductor 宏静默返回，track 宏在 track 不存在时静默返回
//!   （delete 返回空快照，与改造前一致）。

/// conductor 事件 set：定位 `old_tick` 并原地执行 `$body` 修改字段，随后排序。
macro_rules! conductor_set_event {
    ($self:ident, $field:ident, $old_tick:expr, |$event:ident| $body:block) => {
        conductor_set_event!(@imp $self, $field, $old_tick, |$event| $body, false);
    };
    (rebuild tempo_map; $self:ident, $field:ident, $old_tick:expr, |$event:ident| $body:block) => {
        conductor_set_event!(@imp $self, $field, $old_tick, |$event| $body, true);
    };
    (@imp $self:ident, $field:ident, $old_tick:expr, |$event:ident| $body:block, $rebuild:literal) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let Some(idx) = conductor.$field.iter().position(|e| e.tick == $old_tick) else {
            return;
        };
        {
            let $event = &mut conductor.$field[idx];
            $body
        }
        conductor.$field.sort_by_key(|e| e.tick);
        if $rebuild {
            model.rebuild_tempo_map();
        }
        $self.data.bump_revision();
    }};
}

/// conductor 事件批量删除：返回 `(before, after)` 全量快照供 undo。
macro_rules! conductor_delete_events {
    ($self:ident, $field:ident, $ticks:expr) => {
        conductor_delete_events!(@imp $self, $field, $ticks, false)
    };
    (rebuild tempo_map; $self:ident, $field:ident, $ticks:expr) => {
        conductor_delete_events!(@imp $self, $field, $ticks, true)
    };
    (@imp $self:ident, $field:ident, $ticks:expr, $rebuild:literal) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        let before = conductor.$field.clone();
        conductor.$field.retain(|e| !$ticks.contains(&e.tick));
        let after = conductor.$field.clone();
        if $rebuild {
            model.rebuild_tempo_map();
        }
        $self.data.bump_revision();
        (before, after)
    }};
}

/// conductor 事件插入：push 后按 tick 稳定排序。
macro_rules! conductor_insert_event {
    ($self:ident, $field:ident, $event:expr) => {
        conductor_insert_event!(@imp $self, $field, $event, false);
    };
    (rebuild tempo_map; $self:ident, $field:ident, $event:expr) => {
        conductor_insert_event!(@imp $self, $field, $event, true);
    };
    (@imp $self:ident, $field:ident, $event:expr, $rebuild:literal) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let conductor = Arc::make_mut(&mut model.conductor);
        conductor.$field.push($event);
        conductor.$field.sort_by_key(|e| e.tick);
        if $rebuild {
            model.rebuild_tempo_map();
        }
        $self.data.bump_revision();
    }};
}

/// track 事件 set：定位 `old_tick` 并原地执行 `$body` 修改字段，随后排序。
macro_rules! track_set_event {
    ($self:ident, $track:expr, $field:ident, $old_tick:expr, |$event:ident| $body:block) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let Some(td) = model.tracks.get_mut($track as usize) else {
            return;
        };
        let td = Arc::make_mut(td);
        let Some(idx) = td.$field.iter().position(|e| e.tick == $old_tick) else {
            return;
        };
        {
            let $event = &mut td.$field[idx];
            $body
        }
        td.$field.sort_by_key(|e| e.tick);
        $self.data.bump_revision();
    }};
}

/// track 事件批量删除：返回 `(before, after)` 快照；track 不存在时返回空快照。
macro_rules! track_delete_events {
    ($self:ident, $track:expr, $field:ident, $ticks:expr) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let Some(td) = model.tracks.get_mut($track as usize) else {
            return (Vec::new(), Vec::new());
        };
        let td = Arc::make_mut(td);
        let before = td.$field.clone();
        td.$field.retain(|e| !$ticks.contains(&e.tick));
        let after = td.$field.clone();
        $self.data.bump_revision();
        (before, after)
    }};
}

/// track 事件插入：push 后按 tick 稳定排序；track 不存在时静默返回。
macro_rules! track_insert_event {
    ($self:ident, $track:expr, $field:ident, $event:expr) => {{
        let model = Arc::make_mut(&mut $self.data.model);
        let Some(td) = model.tracks.get_mut($track as usize) else {
            return;
        };
        let td = Arc::make_mut(td);
        td.$field.push($event);
        td.$field.sort_by_key(|e| e.tick);
        $self.data.bump_revision();
    }};
}

pub(crate) use {
    conductor_delete_events, conductor_insert_event, conductor_set_event, track_delete_events,
    track_insert_event, track_set_event,
};
