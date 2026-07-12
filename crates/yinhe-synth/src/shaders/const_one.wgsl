// Test: write constant 1.0 to verify pipeline execution
struct RenderParams { frame_count: u32, voice_count: u32, sample_rate: u32, sample_chunk_count: u32 };
@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read_write> voice_states: array<u32>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;
struct ChunkOffsets { o0: u32, o1: u32, o2: u32, o3: u32, o4: u32, total: u32, _pad0: u32, _pad1: u32 };
@group(0) @binding(8) var<uniform> chunk_off: ChunkOffsets;

@compute @workgroup_size(256)
fn vs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let fi = gid.x;
    if fi >= params.frame_count { return; }
    // Write constant 1.0 — proves pipeline works
    final_output[fi * 2u] = 1.0;
    final_output[fi * 2u + 1u] = 1.0;
}
