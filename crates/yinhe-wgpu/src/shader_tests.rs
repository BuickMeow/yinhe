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

/// storage 变体（共享前缀 + storage 前缀 + 主体）必须通过 naga 校验。
#[test]
fn shader_storage_variant_parses_and_validates() {
    validate(crate::pipeline::shader_source(true));
}

/// texture 变体（GLES 3.0 降级路径）同样必须通过 naga 校验。
#[test]
fn shader_texture_variant_parses_and_validates() {
    validate(crate::pipeline::shader_source(false));
}

/// `cull.wgsl`（选中置顶双分区）必须能通过 naga 的解析与校验。
#[test]
fn cull_wgsl_parses_and_validates() {
    validate(include_str!("cull.wgsl"));
}

/// 把所有 entry point 翻译成 GLSL ES 3.00（= wgpu-hal 的 GLES 后端实际走的
/// 编译路径）。GLES 3.0 设备上若这一步失败，运行期只会得到一条
/// 「create_render_pipeline 失败」日志，改动没有可观测的谱面——用测试先兜住。
fn translate_glsl(src: &str, gles_version: u16) {
    let module = naga::front::wgsl::parse_str(src).expect("WGSL 解析失败");
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
        version: naga::back::glsl::Version::new_gles(gles_version),
        writer_flags: naga::back::glsl::WriterFlags::ADJUST_COORDINATE_SPACE,
        binding_map,
        zero_initialize_workgroup_memory: true,
    };

    for entry in &module.entry_points {
        // pipeline override 必须先在 WGSL 里定值（与 wgpu 创建设备时传的
        // `SRGB_TARGET` 常量同一条路径）。
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
        .unwrap_or_else(|e| panic!("{} 翻译 GLSL 失败: {e:?}", entry.name));
        writer
            .write()
            .unwrap_or_else(|e| panic!("{} 生成 GLSL 失败: {e:?}", entry.name));
    }
}

/// storage 变体（GLES 3.1+ / Metal / Vulkan）必须能生成 GLSL ES 3.10。
#[test]
fn shader_storage_variant_translates_to_gles31() {
    translate_glsl(crate::pipeline::shader_source(true), 310);
}

/// texture 变体（GLES 3.0 降级渲染路径）必须能生成 GLSL ES 3.00。
#[test]
fn shader_texture_variant_translates_to_gles3() {
    translate_glsl(crate::pipeline::shader_source(false), 300);
}

#[test]
fn tmp_print_fragment_header() {
    let src = crate::pipeline::shader_source(false);
    let module = naga::front::wgsl::parse_str(src).unwrap();
    let mut v = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let info = v.validate(&module).unwrap();
    let mut map = naga::back::glsl::BindingMap::default();
    for (_, var) in module.global_variables.iter() {
        if let Some(b) = &var.binding {
            let slot = map.len() as u8;
            map.insert(*b, slot);
        }
    }
    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::new_gles(300),
        writer_flags: naga::back::glsl::WriterFlags::ADJUST_COORDINATE_SPACE,
        binding_map: map,
        zero_initialize_workgroup_memory: true,
    };
    for ep in &module.entry_points {
        let mut constants = naga::back::PipelineConstants::default();
        constants.insert("SRGB_TARGET".to_string(), 0.0);
        let (m, i) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            Some((ep.stage, ep.name.as_str())),
            &constants,
        )
        .unwrap();
        let po = naga::back::glsl::PipelineOptions {
            shader_stage: ep.stage,
            entry_point: ep.name.clone(),
            multiview: None,
        };
        let mut out = String::new();
        let mut w =
            naga::back::glsl::Writer::new(&mut out, &m, &i, &options, &po, Default::default())
                .unwrap();
        let _ = w.write();
        println!("{} => {:?}", ep.name, out.lines().next().unwrap());
    }
}
