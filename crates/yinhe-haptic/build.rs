fn main() {
    // Link against the private MultitouchSupport framework to drive the
    // trackpad Taptic Engine directly via MTActuator* APIs.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-search=framework=/System/Library/PrivateFrameworks");
        println!("cargo:rustc-link-lib=framework=MultitouchSupport");
    }
}
