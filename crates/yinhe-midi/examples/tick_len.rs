fn main() {
    for path in std::env::args().skip(1) {
        match yinhe_midi::parse_path(&path) {
            Ok(m) => println!(
                "{} -> ppq={} tick_length={} notes={}",
                path,
                m.meta.ppq,
                m.tick_length,
                m.notes.iter().map(|n| n.len()).sum::<usize>()
            ),
            Err(e) => println!("{} -> ERR {}", path, e),
        }
    }
}
