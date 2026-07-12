/// Generate a vinyl record icon programmatically.
pub fn make_disc_icon() -> egui::IconData {
    let size = 64;
    let cx = size as f32 / 2.0 - 0.5;
    let cy = size as f32 / 2.0 - 0.5;
    let outer_r = (size / 2 - 1) as f32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = ((y * size + x) * 4) as usize;

            if dist <= outer_r {
                if dist < 2.5 {
                    // Center hole -- transparent
                    rgba[idx + 3] = 0;
                } else if dist < 8.0 {
                    // Center label -- reddish
                    rgba[idx] = 220;
                    rgba[idx + 1] = 90;
                    rgba[idx + 2] = 90;
                    rgba[idx + 3] = 255;
                    if dist > 7.0 {
                        let t = ((dist - 7.0) / 1.0).clamp(0.0, 1.0);
                        rgba[idx] = (rgba[idx] as f32 * (1.0 - t)) as u8;
                        rgba[idx + 1] = (rgba[idx + 1] as f32 * (1.0 - t)) as u8;
                        rgba[idx + 2] = (rgba[idx + 2] as f32 * (1.0 - t)) as u8;
                    }
                } else {
                    // Record grooves -- dark gray with subtle rings
                    let groove_val = 25.0 + (dist * 0.4).sin().abs() * 15.0;
                    let v = groove_val.min(255.0) as u8;
                    rgba[idx] = v;
                    rgba[idx + 1] = v;
                    rgba[idx + 2] = v;
                    rgba[idx + 3] = 255;
                }
            }
        }
    }

    egui::IconData { rgba, width: size, height: size }
}
