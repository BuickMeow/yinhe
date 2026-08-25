//! 实验：按 (track, start, key) 轨道串行排序（模拟 MIDI 轨道布局），交错写流。
//! 用法: cargo run --release -p yinhe-yin --example track_serial -- <dir>

fn push_varint(out: &mut Vec<u8>, v: u64) {
    if v <= 250 {
        out.push(v as u8);
    } else if v <= u16::MAX as u64 {
        out.push(251);
        out.extend_from_slice(&(v as u16).to_le_bytes());
    } else if v <= u32::MAX as u64 {
        out.push(252);
        out.extend_from_slice(&(v as u32).to_le_bytes());
    } else {
        out.push(253);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/tmp/yin_exp".to_string());
    let split = args.get(2).map(|s| s.as_str()) == Some("--split");
    let read_bin = |name: &str| std::fs::read(format!("{dir}/{name}")).expect(name);

    let t = std::time::Instant::now();
    let delta: Vec<u32> = postcard::from_bytes(&read_bin("l3_delta.bin")).unwrap();
    let key: Vec<u8> = postcard::from_bytes(&read_bin("l3_key.bin")).unwrap();
    let track: Vec<u16> = postcard::from_bytes(&read_bin("l3_track.bin")).unwrap();
    let vel: Vec<u8> = postcard::from_bytes(&read_bin("l3_vel.bin")).unwrap();
    let gate: Vec<u32> = postcard::from_bytes(&read_bin("l3_gate.bin")).unwrap();
    let n = delta.len();
    assert_eq!(key.len(), n);
    assert_eq!(track.len(), n);
    assert_eq!(vel.len(), n);
    assert_eq!(gate.len(), n);

    // 还原绝对 start
    let mut start = Vec::with_capacity(n);
    let mut prev = 0u32;
    for d in &delta {
        prev += d;
        start.push(prev);
    }
    println!("解码: {:?}", t.elapsed());

    // 计数排序 by track（稳定）：先统计每轨数量
    let max_track = *track.iter().max().unwrap() as usize + 1;
    let t = std::time::Instant::now();
    let mut counts = vec![0u64; max_track];
    for &t_ in &track {
        counts[t_ as usize] += 1;
    }
    // 轨道内还需要按 (start, key) 排序：直接对每轨的音符做一次完整索引排序
    // 简化：构造 64bit 排序键 (track<<32|start) 不行，key 也要。用稳定计数排序
    // 两次：先按 track 分桶（计数排序），桶内已保持 start 全局序（因为原流按 start 有序！）
    // 原流按 (start, track, key) 有序 → 按 track 稳定分桶后，桶内自然按 (start, key) 有序！
    let mut offsets = vec![0u64; max_track + 1];
    for i in 0..max_track {
        offsets[i + 1] = offsets[i] + counts[i];
    }
    let mut order = vec![0u32; n];
    let mut cursor = offsets.clone();
    for i in 0..n as u32 {
        let t_ = track[i as usize] as usize;
        order[cursor[t_] as usize] = i;
        cursor[t_] += 1;
    }
    println!("计数排序: {:?} (track 数 {max_track})", t.elapsed());

    // 轨道串行交错输出：[轨内 delta][key][vel][gate]（track 隐含在段内）
    let t = std::time::Instant::now();
    let split_dir = format!("{dir}/track_split");
    std::fs::create_dir_all(&split_dir).expect("mkdir");
    let mut out = Vec::with_capacity(n * 6);
    let mut split_out: Vec<Vec<u8>> = Vec::new(); // 每轨一个缓冲
    let mut prev_start_in_track = 0u32;
    let mut cur_track = usize::MAX;
    let mut cur_buf: Option<Vec<u8>> = None;
    for &idx in &order {
        let i = idx as usize;
        let tr = track[i] as usize;
        if tr != cur_track {
            cur_track = tr;
            prev_start_in_track = 0;
            push_varint(&mut out, tr as u64);
            if let Some(b) = cur_buf.take() {
                split_out.push(b);
            }
            cur_buf = Some(Vec::new());
        }
        let d = if prev_start_in_track == 0 {
            start[i]
        } else {
            start[i] - prev_start_in_track
        };
        push_varint(&mut out, d as u64);
        out.push(key[i]);
        out.push(vel[i]);
        push_varint(&mut out, gate[i] as u64);
        if let Some(b) = cur_buf.as_mut() {
            push_varint(b, d as u64);
            b.push(key[i]);
            b.push(vel[i]);
            push_varint(b, gate[i] as u64);
        }
        prev_start_in_track = start[i];
    }
    if let Some(b) = cur_buf.take() {
        split_out.push(b);
    }
    let path = format!("{dir}/track_serial.bin");
    std::fs::write(&path, &out).expect("write");
    println!(
        "track_serial.bin: {:.2} MiB, 耗时 {:?}",
        out.len() as f64 / (1u64 << 20) as f64,
        t.elapsed()
    );
    if split {
        let mut total = 0usize;
        for (tr, b) in split_out.iter().enumerate() {
            std::fs::write(format!("{split_dir}/tr{tr:04}.bin"), b).expect("write split");
            total += b.len();
        }
        println!(
            "split: {} 轨, 原始合计 {:.2} MiB",
            split_out.len(),
            total as f64 / (1u64 << 20) as f64
        );
    }
}
