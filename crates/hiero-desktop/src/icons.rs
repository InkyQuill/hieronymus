use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use std::io::Cursor;

const SUPPORTED_SIZES: [u32; 13] = [16, 20, 22, 24, 32, 40, 44, 48, 64, 128, 256, 512, 1024];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    foreground: [u8; 3],
    accent: [u8; 3],
    size: u32,
}

static ICON_CACHE: OnceLock<Mutex<HashMap<CacheKey, Vec<u8>>>> = OnceLock::new();

/// Renders one semantic icon as straight-alpha RGBA pixels.
pub fn render_icon(foreground: [u8; 3], accent: [u8; 3], size: u32) -> Result<Vec<u8>, String> {
    if !SUPPORTED_SIZES.contains(&size) {
        return Err(format!("unsupported icon size {size}"));
    }

    let key = CacheKey {
        foreground,
        accent,
        size,
    };
    let cache = ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(rgba) = cache
        .lock()
        .map_err(|_| "icon cache lock is poisoned".to_owned())?
        .get(&key)
        .cloned()
    {
        return Ok(rgba);
    }

    let rgba = recolor_mask(foreground, accent, size)?;
    cache
        .lock()
        .map_err(|_| "icon cache lock is poisoned".to_owned())?
        .insert(key, rgba.clone());
    Ok(rgba)
}

// The grayscale channel is foreground coverage within straight-alpha pixels;
// black is the accent region. Geometry is frozen, while palettes stay dynamic.
fn recolor_mask(foreground: [u8; 3], accent: [u8; 3], size: u32) -> Result<Vec<u8>, String> {
    let bytes: &[u8] = match size {
        16 => include_bytes!("../../../assets/icons/native/masks/16.png"),
        20 => include_bytes!("../../../assets/icons/native/masks/20.png"),
        22 => include_bytes!("../../../assets/icons/native/masks/22.png"),
        24 => include_bytes!("../../../assets/icons/native/masks/24.png"),
        32 => include_bytes!("../../../assets/icons/native/masks/32.png"),
        40 => include_bytes!("../../../assets/icons/native/masks/40.png"),
        44 => include_bytes!("../../../assets/icons/native/masks/44.png"),
        48 => include_bytes!("../../../assets/icons/native/masks/48.png"),
        64 => include_bytes!("../../../assets/icons/native/masks/64.png"),
        128 => include_bytes!("../../../assets/icons/native/masks/128.png"),
        256 => include_bytes!("../../../assets/icons/native/masks/256.png"),
        512 => include_bytes!("../../../assets/icons/native/masks/512.png"),
        1024 => include_bytes!("../../../assets/icons/native/masks/1024.png"),
        _ => return Err(format!("unsupported icon size {size}")),
    };
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|e| format!("invalid icon mask: {e}"))?;
    let length = reader
        .output_buffer_size()
        .ok_or("invalid icon mask dimensions")?;
    if length != (size * size * 4) as usize {
        return Err("invalid icon mask dimensions".into());
    }
    let mut rgba = vec![0; length];
    let info = reader
        .next_frame(&mut rgba)
        .map_err(|e| format!("invalid icon mask: {e}"))?;
    if info.width != size
        || info.height != size
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err("invalid icon mask format".into());
    }
    for pixel in rgba.as_chunks_mut::<4>().0.iter_mut() {
        let coverage = u32::from(pixel[0]);
        for channel in 0..3 {
            pixel[channel] = if pixel[3] == 0 {
                0
            } else {
                ((coverage * u32::from(foreground[channel])
                    + (255 - coverage) * u32::from(accent[channel])
                    + 127)
                    / 255) as u8
            };
        }
    }
    Ok(rgba)
}
