//! Exact pixel boundary between the shared straight-RGBA cache and Win32 DIBs.
pub fn convert(rgba: &[u8], size: u32) -> Result<(Vec<u8>, Vec<u8>), String> {
    if size == 0 || size > 1024 || rgba.len() != size as usize * size as usize * 4 {
        return Err("Invalid native icon dimensions".into());
    }
    let mut bgra = rgba.to_vec();
    for (dest, src) in bgra.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let a = src[3] as u16;
        dest.copy_from_slice(&[
            (src[2] as u16 * a / 255) as u8,
            (src[1] as u16 * a / 255) as u8,
            (src[0] as u16 * a / 255) as u8,
            src[3],
        ]);
    }
    let stride = (size as usize).div_ceil(32) * 4;
    let mut mask = vec![0; stride * size as usize];
    for (index, pixel) in rgba.chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            let x = index % size as usize;
            let y = index / size as usize;
            mask[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    Ok((bgra, mask))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_alpha_channel_order_and_transparent_mask_are_exact() {
        let source = [
            200, 100, 50, 128, 10, 20, 30, 255, 99, 88, 77, 0, 250, 200, 100, 64,
        ];
        let (bgra, mask) = convert(&source, 2).unwrap();
        assert_eq!(
            bgra,
            [
                25, 50, 100, 128, 30, 20, 10, 255, 0, 0, 0, 0, 25, 50, 62, 64
            ]
        );
        assert_eq!(mask, [0, 0, 0, 0, 0x80, 0, 0, 0]);
        assert_eq!(source[0], 200); // cache input remains straight RGBA
    }
    #[test]
    fn invalid_sizes_never_reach_native_memory_copy() {
        assert!(convert(&[], 0).is_err());
        assert!(convert(&[0; 3], 1).is_err());
        assert!(convert(&[], u32::MAX).is_err());
    }
}
