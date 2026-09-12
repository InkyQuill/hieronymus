use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

const TRAY_SVG: &str = include_str!("../../../assets/icons/icon-tray.svg");
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

    let rgba = render_uncached(TRAY_SVG, foreground, accent, size)?;
    cache
        .lock()
        .map_err(|_| "icon cache lock is poisoned".to_owned())?
        .insert(key, rgba.clone());
    Ok(rgba)
}

fn render_uncached(
    source: &str,
    foreground: [u8; 3],
    accent: [u8; 3],
    size: u32,
) -> Result<Vec<u8>, String> {
    let svg = color_regions(source, foreground, accent)?;
    let tree = Tree::from_str(&svg, &Options::default())
        .map_err(|error| format!("invalid tray SVG: {error}"))?;
    let mut pixmap = Pixmap::new(size, size)
        .ok_or_else(|| format!("could not allocate {size}x{size} icon surface"))?;
    let scale = size as f32 / 75.0;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let mut rgba = pixmap.take();
    unpremultiply(&mut rgba);
    Ok(rgba)
}

fn color_regions(source: &str, foreground: [u8; 3], accent: [u8; 3]) -> Result<String, String> {
    let foreground = replace_region(source, "hiero-foreground", "currentColor", foreground)?;
    replace_region(&foreground, "hiero-accent", "#a45734", accent)
}

fn replace_region(
    source: &str,
    id: &str,
    expected_fill: &str,
    color: [u8; 3],
) -> Result<String, String> {
    let marker = format!("id=\"{id}\"");
    let matches: Vec<_> = source.match_indices(&marker).collect();
    if matches.len() != 1 {
        return Err(format!(
            "tray SVG must contain exactly one {marker}; found {}",
            matches.len()
        ));
    }

    let marker_start = matches[0].0;
    let tag_start = source[..marker_start]
        .rfind('<')
        .ok_or_else(|| format!("region {id} is outside an SVG element"))?;
    let tag_end = marker_start
        + source[marker_start..]
            .find('>')
            .ok_or_else(|| format!("region {id} has an unterminated SVG element"))?;
    let tag = &source[tag_start..=tag_end];
    if !tag.starts_with("<path ") {
        return Err(format!("region {id} must identify a path element"));
    }

    let fill_marker = "fill=\"";
    let fill_matches: Vec<_> = tag.match_indices(fill_marker).collect();
    if fill_matches.len() != 1 {
        return Err(format!(
            "region {id} must contain exactly one fill attribute; found {}",
            fill_matches.len()
        ));
    }
    let value_start = tag_start + fill_matches[0].0 + fill_marker.len();
    let value_end = value_start
        + source[value_start..]
            .find('"')
            .ok_or_else(|| format!("region {id} has an unterminated fill attribute"))?;
    if &source[value_start..value_end] != expected_fill {
        return Err(format!(
            "region {id} must use fill {expected_fill}; found {}",
            &source[value_start..value_end]
        ));
    }

    let replacement = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
    let mut colored = source.to_owned();
    colored.replace_range(value_start..value_end, &replacement);
    Ok(colored)
}

fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
            continue;
        }
        for channel in &mut pixel[..3] {
            let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;
            *channel = straight.min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TRAY_SVG, color_regions};

    #[test]
    fn missing_region_is_rejected() {
        let malformed = TRAY_SVG.replace("id=\"hiero-accent\"", "id=\"other\"");
        let error = color_regions(&malformed, [1, 2, 3], [4, 5, 6]).unwrap_err();
        assert!(error.contains("found 0"), "{error}");
    }

    #[test]
    fn duplicate_region_is_rejected() {
        let malformed = TRAY_SVG.replace(
            "</svg>",
            "  <path id=\"hiero-foreground\" fill=\"currentColor\" d=\"M0 0z\"/>\n</svg>",
        );
        let error = color_regions(&malformed, [1, 2, 3], [4, 5, 6]).unwrap_err();
        assert!(error.contains("found 2"), "{error}");
    }

    #[test]
    fn unexpected_region_fill_is_rejected() {
        let malformed = TRAY_SVG.replace("fill=\"currentColor\"", "fill=\"#ffffff\"");
        let error = color_regions(&malformed, [1, 2, 3], [4, 5, 6]).unwrap_err();
        assert!(error.contains("must use fill currentColor"), "{error}");
    }
}
