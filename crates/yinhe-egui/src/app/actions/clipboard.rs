//! 复制 / 剪切 / 粘贴（含系统剪贴板与 Domino 互通）。

use rust_i18n::t;

use crate::app::{App, PasteChain};

impl App {
    // ── Copy / Cut / Paste / Select All ──

    /// Copy selected notes to the clipboard as an O(1) model snapshot.
    /// Empty selection leaves the clipboard untouched (成熟软件惯例：空复制不覆盖).
    pub(crate) fn copy_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &self.workspace.documents[idx];
        if doc.edit.selected.is_empty() {
            return;
        }
        self.clipboard = yinhe_editor_core::ClipboardContent::Notes(
            yinhe_editor_core::NotesClipboard::from_snapshot(
                doc.data.model.clone(),
                doc.edit.selected.clone(),
            ),
        );
        self.paste_chain = None;
        self.export_clipboard_to_system();
    }

    /// Cut: copy selection (snapshot), then delete selected notes.
    pub(crate) fn cut_selection(&mut self) {
        self.copy_selection();
        self.delete_selected_notes();
    }

    /// Paste the clipboard at cursor position, dispatching on clipboard content
    /// (notes vs automation), not on the current selection.
    pub(crate) fn paste_clipboard(&mut self) {
        self.paste_clipboard_with_mode(yinhe_editor_core::clipboard::PasteMode::AtCursor);
    }

    /// Paste with an explicit placement mode.
    ///
    /// `AtCursor` participates in the consecutive-paste chain: pressing paste
    /// again without moving the cursor advances by the pasted content span.
    pub(crate) fn paste_clipboard_with_mode(
        &mut self,
        mode: yinhe_editor_core::clipboard::PasteMode,
    ) {
        use yinhe_editor_core::clipboard::PasteMode;

        let Some(idx) = self.workspace.active_doc else {
            return;
        };

        // 系统剪贴板是剪贴板内容的唯一真相：外部内容使内部剪贴板失效；
        // 其他实例的引用则从临时文件加载。已是最新内容时零开销。
        if !self.clipboard_sync.resolve_paste(&mut self.clipboard)
            && !self.load_domino_clipboard(idx)
        {
            return;
        }

        let clipboard = self.clipboard.clone();
        let chained = mode == PasteMode::AtCursor;
        let cursor_tick = self.workspace.documents[idx]
            .edit
            .cursor_tick
            .unwrap_or(0.0);

        match clipboard {
            yinhe_editor_core::ClipboardContent::Empty => {}
            yinhe_editor_core::ClipboardContent::Notes(cb) => {
                let chain_offset = if chained {
                    self.paste_chain_offset(cursor_tick)
                } else {
                    0
                };
                let effective_cursor = cursor_tick + chain_offset as f64;
                let track_selected = self.workspace.documents[idx].edit.track_selected.clone();
                let span = self.with_undo_result(t!("undo.paste").as_ref(), |doc| {
                    doc.paste_notes(&cb, effective_cursor, &track_selected, mode)
                });
                if let Some(span) = span
                    && chained
                {
                    self.advance_paste_chain(cursor_tick, span.max(1));
                }
            }
            yinhe_editor_core::ClipboardContent::Automation(cb) => {
                let span = self.paste_automation_clipboard(&cb, mode);
                if let Some(span) = span
                    && chained
                {
                    let advance = self.automation_paste_advance(span);
                    self.advance_paste_chain(cursor_tick, advance);
                }
            }
        }
    }

    /// 系统剪贴板里是 Domino `MidiPortalSequence` 时，解码成 yinhe 音符剪贴板。
    /// 返回 false = 没有 Domino 数据或解析失败（调用方按普通外部剪贴板处理）。
    fn load_domino_clipboard(&mut self, idx: usize) -> bool {
        let Some(raw) = self.clipboard_sync.domino_payload() else {
            return false;
        };
        let clip = match yinhe_editor_core::domino_clipboard::decode_domino_clipboard(&raw) {
            Ok(clip) => clip,
            Err(e) => {
                tracing::debug!("Domino 剪贴板解码失败: {e}");
                self.notifications
                    .error(t!("toast.clipboard_paste_failed"), e);
                return false;
            }
        };
        let doc = &self.workspace.documents[idx];
        let Some(track) = doc.edit.write_track() else {
            return false;
        };
        let ppq = doc.data.model.meta.ppq;
        let note_count: usize = clip.tracks.iter().map(Vec::len).sum();
        let track_count = doc.data.model.tracks.len();
        self.clipboard = yinhe_editor_core::ClipboardContent::Notes(
            yinhe_editor_core::domino_clipboard::to_notes_clipboard(&clip, ppq, track, track_count),
        );
        // Domino 数据是相对复制起点的 tick；粘贴时按 yinhe 惯例对齐到光标；
        // 不要接上一次连续粘贴链的偏移，否则第二次粘贴会莫名向后跳。
        self.paste_chain = None;
        tracing::info!(
            "已从 Domino 剪贴板载入 {} 个音符 / {} 轨（{} PPQ → {} PPQ）",
            note_count,
            clip.tracks.len(),
            clip.ppq,
            ppq
        );
        true
    }

    /// 连续粘贴链的当前偏移（光标未动时非零）。
    fn paste_chain_offset(&self, cursor_tick: f64) -> u32 {
        match self.paste_chain {
            Some(chain) if chain.cursor_tick == cursor_tick => chain.offset,
            _ => 0,
        }
    }

    /// 记录本次粘贴的内容跨度，下一次同光标粘贴将自动递增。
    fn advance_paste_chain(&mut self, cursor_tick: f64, advance: u32) {
        let advance = advance.max(1);
        match &mut self.paste_chain {
            Some(chain) if chain.cursor_tick == cursor_tick => {
                chain.offset = chain.offset.saturating_add(advance);
            }
            _ => {
                self.paste_chain = Some(PasteChain {
                    cursor_tick,
                    offset: advance,
                });
            }
        }
    }

    /// 自动化连续粘贴的递进量：多锚点用内容跨度；单锚点跨度 0 时用量化间隔。
    fn automation_paste_advance(&self, span: u32) -> u32 {
        if span > 0 {
            return span;
        }
        let Some(idx) = self.workspace.active_doc else {
            return 1;
        };
        let doc = &self.workspace.documents[idx];
        let ppq = doc.data.model.meta.ppq;
        let (sig_num, sig_den) = doc.data.model.tempo_map.time_sig_default;
        doc.edit
            .quantize_pianoroll
            .resolved_with_time_sig(
                doc.edit.pianoroll_view.base.pixels_per_tick,
                ppq,
                sig_num,
                sig_den,
                true,
            )
            .tick_interval(ppq)
            .max(1)
    }
}
