use crate::document::Document;

/// 多文档工作区（纯业务，无 egui 依赖）。
/// 从 `yinhe-egui/src/app.rs` 的 `App` 上帝对象中抽离，聚焦文档栈管理。
/// `App` 仅保留渲染/音频/UI 状态，文档操作通过 `workspace` 代理。
pub struct Workspace {
    pub documents: Vec<Document>,
    pub active_doc: Option<usize>,
    pub prev_active_doc: Option<usize>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            documents: vec![Document::empty()],
            active_doc: Some(0),
            prev_active_doc: Some(0),
        }
    }
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// 判断打开 MIDI/.yin 时是否应替换当前标签页而非另开一个。
    /// 仅当当前是首次启动的 Untitled（`documents.len() == 1`、活跃在 idx 0、
    /// `file_path.is_none()` 且未修改）时返回 true。
    pub fn should_replace_initial_untitled(&self) -> bool {
        if self.documents.len() != 1 || self.active_doc != Some(0) {
            return false;
        }
        let doc = &self.documents[0];
        doc.file_path.is_none()
            && doc.model().note_count == 0
            && !doc.history.is_dirty()
            && !doc.mixer_dirty
    }

    /// 新建空白工程并切换为活跃标签页。
    /// 调用方需同步 `mixer_racks` / `instrument_racks` 等平行数组（egui 层）。
    pub fn new_project(&mut self) -> usize {
        self.documents.push(Document::empty());
        let idx = self.documents.len() - 1;
        self.active_doc = Some(idx);
        idx
    }

    /// 标题栏标签拖动排序：把 from 位置的工程移动到剩余列表的 insert_at 位置。
    /// 返回新的 active 索引（若有）。
    pub fn reorder_tab(&mut self, from: usize, insert_at: usize) -> Option<usize> {
        let len = self.documents.len();
        if from >= len {
            return self.active_doc;
        }
        let order = reorder::plan_order(len, &[from], insert_at);
        let cur: Vec<usize> = (0..len).collect();
        if order == cur {
            return self.active_doc;
        }
        let old_active = self.active_doc;
        reorder::apply_reorder_noclone(&mut self.documents, &[from], insert_at);
        if let Some(active) = old_active {
            self.active_doc = order.iter().position(|&i| i == active);
        }
        // 只是索引重排，文档内容未变：对齐 prev 避免下一帧误触发切换检测
        self.prev_active_doc = self.active_doc;
        self.active_doc
    }

    /// 移除指定索引的文档，修正 active 索引，返回被移除的文档。
    /// 调用方需同步处理 `mixer_racks` 等平行数组及音频 teardown。
    pub fn take_document(&mut self, index: usize) -> Option<Document> {
        if index >= self.documents.len() {
            return None;
        }
        let doc = self.documents.remove(index);
        if let Some(active) = self.active_doc {
            if index < active {
                self.active_doc = Some(active - 1);
            } else if index == active {
                self.active_doc = if self.documents.is_empty() {
                    None
                } else {
                    Some(active.min(self.documents.len() - 1))
                };
            }
        }
        Some(doc)
    }

    /// 关闭指定索引的文档（`take_document` 的薄壳，语义更清晰）。
    pub fn close_document(&mut self, index: usize) -> Option<Document> {
        self.take_document(index)
    }

    /// 当前活跃文档的可变引用。
    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        self.active_doc.and_then(|i| self.documents.get_mut(i))
    }

    /// 当前活跃文档的不可变引用。
    pub fn active_doc_ref(&self) -> Option<&Document> {
        self.active_doc.and_then(|i| self.documents.get(i))
    }
}

// ── 轻量排序辅助（原 widgets::reorder，下沉至此避免 egui 依赖） ──
pub mod reorder {
    /// 计算拖拽排序后的最终顺序：被拖行整体移动到
    /// "删除它们后的列表中的 `insert_at` 位置"，其余行保持原顺序。
    pub fn plan_order(len: usize, indices: &[usize], insert_at: usize) -> Vec<usize> {
        let insert_at = insert_at.min(len.saturating_sub(indices.len()));
        let remaining: Vec<usize> = (0..len).filter(|i| !indices.contains(i)).collect();
        let mut order = Vec::with_capacity(len);
        order.extend_from_slice(&remaining[..insert_at]);
        order.extend_from_slice(indices);
        order.extend_from_slice(&remaining[insert_at..]);
        order
    }

    /// 对 Vec 直接应用排序（不要求 Clone，用于 Document / MixerRack 等）。
    pub fn apply_reorder_noclone<T>(items: &mut Vec<T>, indices: &[usize], insert_at: usize) {
        let order = plan_order(items.len(), indices, insert_at);
        let mut old: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
        let mut new = Vec::with_capacity(order.len());
        for &idx in &order {
            new.push(old[idx].take().expect("order 是排列"));
        }
        *items = new;
    }
}
