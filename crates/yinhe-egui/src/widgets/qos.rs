/// Execute the closure with temporarily lowered QoS on macOS so the
/// operating system scheduler favours the audio real-time thread.
#[cfg(target_os = "macos")]
pub fn guarded<T>(f: impl FnOnce() -> T) -> T {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u64, relative_priority: i32) -> i32;
    }

    const QOS_CLASS_USER_INTERACTIVE: u64 = 0x21;
    const QOS_CLASS_UTILITY: u64 = 0x11;

    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
    let result = f();
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
    }
    result
}

#[cfg(not(target_os = "macos"))]
pub fn guarded<T>(f: impl FnOnce() -> T) -> T {
    f()
}
