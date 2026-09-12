//! NSImage stays 18 logical points; select a supported shared SVG raster size.
pub fn raster_size(scale: f64) -> u32 {
    let scale = if scale.is_finite() {
        scale.clamp(1.0, 4.0)
    } else {
        1.0
    };
    let required = (18.0 * scale).ceil() as u32;
    [20, 22, 24, 32, 40, 44, 48, 64, 128]
        .into_iter()
        .find(|size| *size >= required)
        .unwrap_or(128)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_backing_scale_uses_a_supported_color_raster() {
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
            let size = raster_size(scale);
            assert!(f64::from(size) >= 18.0 * scale);
            let rgba = crate::render_icon([24; 3], [46, 173, 104], size).unwrap();
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert!(rgba.chunks_exact(4).any(|p| p[3] > 0));
        }
    }
}
