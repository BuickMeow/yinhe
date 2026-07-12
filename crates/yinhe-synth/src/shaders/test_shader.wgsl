// Debug shader: output the binary search result
struct RenderParams { frame_count: u32, voice_count: u32, sample_rate: u32, sample_chunk_count: u32 };
@group(0) @binding(0) var<uniform> params: RenderParams;
@group(0) @binding(1) var<storage, read_write> voice_states: array<u32>;
@group(0) @binding(2) var<storage, read_write> final_output: array<f32>;
@group(0) @binding(3) var<storage, read> chunk_0: array<f32>;
@group(0) @binding(4) var<storage, read> chunk_1: array<f32>;
@group(0) @binding(5) var<storage, read> chunk_2: array<f32>;
@group(0) @binding(6) var<storage, read> chunk_3: array<f32>;
@group(0) @binding(7) var<storage, read> chunk_4: array<f32>;
struct ChunkOffsets { o0: u32, o1: u32, o2: u32, o3: u32, o4: u32, total: u32 };
@group(0) @binding(8) var<uniform> chunk_off: ChunkOffsets;

@compute @workgroup_size(256)
fn vs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let fi = gid.x;
    if fi >= params.frame_count { return; }
    // Directly read chunk_0[fi] — bypass sample_at
    let val = chunk_0[fi];
    final_output[fi * 2u] = val;
    final_output[fi * 2u + 1u] = val;
    // Also write chunk_off.o0 and sample_chunk_count to debug
    // Pack into first two frames
    if fi == 0u {
        final_output[0u] = f32(params.sample_chunk_count);
        final_output[1u] = f32(chunk_off.o0);
    }
}
