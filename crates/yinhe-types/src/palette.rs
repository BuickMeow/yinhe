/// Predefined color palette for tracks (up to 16 tracks, cycles for more).
///
/// Shared across egui UI (track panel) and GPU renderer (pianoroll notes).
pub const TRACK_PALETTE: [[f32; 3]; 16] = [
    [0.29, 0.56, 0.89], // blue
    [0.89, 0.35, 0.35], // red
    [0.30, 0.78, 0.30], // green
    [0.95, 0.65, 0.20], // orange
    [0.65, 0.40, 0.85], // purple
    [0.20, 0.80, 0.80], // cyan
    [0.95, 0.75, 0.20], // yellow
    [0.90, 0.45, 0.70], // pink
    [0.40, 0.65, 0.35], // olive
    [0.70, 0.50, 0.30], // brown
    [0.35, 0.55, 0.75], // steel
    [0.85, 0.55, 0.35], // copper
    [0.45, 0.80, 0.55], // mint
    [0.75, 0.35, 0.55], // wine
    [0.55, 0.55, 0.80], // lavender
    [0.60, 0.75, 0.30], // lime
];
