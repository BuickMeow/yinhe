//! WGSL 静态校验：解析并验证 `shader.wgsl`，让改 shader 后的语法/类型错误在
//! `cargo test` 阶段暴露，而不是到运行时创建 pipeline 才 panic（无 GPU 也能跑）。

/// `shader.wgsl` 必须能通过 naga 的解析与校验。
#[test]
fn shader_wgsl_parses_and_validates() {
    let src = include_str!("shader.wgsl");
    let module = naga::front::wgsl::parse_str(src).expect("WGSL 解析失败");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module).expect("WGSL 校验失败");
}
