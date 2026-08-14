package com.jieneng.yinhe;

import android.view.inputmethod.EditorInfo;

import com.google.androidgamesdk.GameActivity;
import com.google.androidgamesdk.gametextinput.GameTextInput;
import com.google.androidgamesdk.gametextinput.InputConnection;
import com.google.androidgamesdk.gametextinput.Settings;

/**
 * GameActivity 的输入定制层：默认 InputEnabledSurfaceView 只在 GameTextInput
 * "激活"时才返回 InputConnection，而 winit 在 egui 焦点闪断（点击输入框瞬间
 * 清空再设置）时会 set_ime_allowed(false) 把激活状态关掉，导致 IMM 解绑、
 * 键盘弹出即收起。这里换用始终返回连接的 surface，绑定保持稳定。
 *
 * 文本回流：winit 0.30 安卓后端不转发输入法事件（TextEvent 被静默丢弃），
 * 输入法文本经本 surface 的 InputConnection 进入 GameTextInput 后，由
 * MainActivity 覆写 stateChanged 回调经 JNI 回流 Rust（见 MainActivity.kt /
 * crates/yinhe-android/src/ime.rs）。
 */
public class YinheActivity extends GameActivity {

    public class YinheInputSurfaceView extends GameActivity.InputEnabledSurfaceView {

        private final InputConnection connection;

        public YinheInputSurfaceView(GameActivity activity) {
            activity.super(activity);
            connection = new InputConnection(
                    activity,
                    this,
                    // 不转发按键事件：硬件键盘事件走 winit 的 KeyEvent 链路。
                    new Settings(activity.getImeEditorInfo(), false));
            connection.setListener(activity);
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo editorInfo) {
            // 同步当前 EditorInfo（inputType / imeOptions / NO_EXTRACT_UI）。
            GameTextInput.copyEditorInfo(YinheActivity.this.getImeEditorInfo(), editorInfo);
            return connection;
        }

        /** MainActivity 经 mSurfaceView 取输入连接（光标回推/文本同步用）。 */
        public InputConnection getConnection() {
            return connection;
        }
    }

    @Override
    protected InputEnabledSurfaceView createSurfaceView() {
        return new YinheInputSurfaceView(this);
    }
}
