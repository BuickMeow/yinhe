//! WGSL 静态校验：解析并验证 shader，让改 shader 后的语法/类型错误在
//! `cargo test` 阶段暴露，而不是到运行时创建 pipeline 才 panic（无 GPU 也能跑）。

fn validate(src: &str) {
    let module = naga::front::wgsl::parse_str(src).expect("WGSL 解析失败");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module).expect("WGSL 校验失败");
}

/// `shader.wgsl` 必须能通过 naga 的解析与校验。
#[test]
fn shader_wgsl_parses_and_validates() {
    validate(include_str!("shader.wgsl"));
}

/// `cull.wgsl`（选中置顶双分区）必须能通过 naga 的解析与校验。
#[test]
fn cull_wgsl_parses_and_validates() {
    validate(include_str!("cull.wgsl"));
}

/// 把 shader 的每个 entry point 翻译成 GLSL ES 3.20（Android 真机 GLES 后端
/// 的编译路径）。naga 侧的类型/版本问题在 `cargo test` 就能暴露，不必等真机。
#[test]
fn shader_translates_to_gles3() {
    let module = naga::front::wgsl::parse_str(include_str!("shader.wgsl")).expect("WGSL 解析失败");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let info = validator.validate(&module).expect("WGSL 校验失败");

    // wgpu-hal 的 GLES 后端把每个 (group, binding) 映射到一个 GL 资源槽位。
    let mut binding_map = naga::back::glsl::BindingMap::default();
    for (_, var) in module.global_variables.iter() {
        if let Some(binding) = &var.binding {
            let slot = binding_map.len() as u8;
            binding_map.insert(*binding, slot);
        }
    }
    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::new_gles(320),
        writer_flags: naga::back::glsl::WriterFlags::ADJUST_COORDINATE_SPACE,
        binding_map,
        zero_initialize_workgroup_memory: true,
    };

    for entry in &module.entry_points {
        // pipeline override 先定值（与 wgpu 创建设备时传的 SRGB_TARGET 一致）。
        let mut constants = naga::back::PipelineConstants::default();
        constants.insert("SRGB_TARGET".to_string(), 1.0);
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            Some((entry.stage, entry.name.as_str())),
            &constants,
        )
        .unwrap_or_else(|e| panic!("{} 覆盖常量失败: {e:?}", entry.name));
        let pipeline_options = naga::back::glsl::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
            multiview: None,
        };
        let mut out = String::new();
        let mut writer = naga::back::glsl::Writer::new(
            &mut out,
            &module,
            &info,
            &options,
            &pipeline_options,
            naga::proc::BoundsCheckPolicies::default(),
        )
        .unwrap_or_else(|e| panic!("{} 翻译 GLSL ES 3.20 失败: {e:?}", entry.name));
        writer
            .write()
            .unwrap_or_else(|e| panic!("{} 生成 GLSL ES 3.20 失败: {e:?}", entry.name));
    }
}
