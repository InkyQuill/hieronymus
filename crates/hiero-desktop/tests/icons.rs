use hiero_desktop::render_icon;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;

const STATUS_SIZES: [u32; 9] = [16, 20, 22, 24, 32, 40, 44, 48, 64];
const APP_SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];

#[test]
fn icon_has_transparent_padding_and_both_regions() {
    let rgba = render_icon([240, 240, 240], [46, 173, 104], 24).unwrap();

    assert_eq!(rgba.len(), 24 * 24 * 4);
    assert_eq!(rgba[3], 0);
    assert!(
        rgba.chunks_exact(4)
            .any(|pixel| pixel == [46, 173, 104, 255])
    );
    assert!(
        rgba.chunks_exact(4)
            .any(|pixel| pixel == [240, 240, 240, 255])
    );
}

#[test]
fn antialiased_pixels_use_straight_alpha() {
    let rgba = render_icon([255, 255, 255], [255, 0, 0], 24).unwrap();

    assert!(rgba.chunks_exact(4).any(|pixel| {
        let alpha = pixel[3];
        alpha > 0 && alpha < 255 && pixel[..3].iter().any(|channel| *channel > alpha)
    }));
}

#[test]
fn palette_changes_preserve_the_silhouette() {
    let light = render_icon([16, 33, 52], [46, 173, 104], 22).unwrap();
    let dark = render_icon([240, 240, 240], [217, 74, 72], 22).unwrap();

    let light_alpha: Vec<_> = light.chunks_exact(4).map(|pixel| pixel[3]).collect();
    let dark_alpha: Vec<_> = dark.chunks_exact(4).map(|pixel| pixel[3]).collect();
    assert_eq!(light_alpha, dark_alpha);
}

#[test]
fn documented_status_and_app_dimensions_render() {
    for size in STATUS_SIZES.into_iter().chain(APP_SIZES) {
        let rgba = render_icon([16, 33, 52], [164, 87, 52], size).unwrap();
        assert_eq!(rgba.len(), (size * size * 4) as usize);
    }
}

#[test]
fn undocumented_dimensions_are_rejected() {
    for size in [0, 15, 18, 75, 2048] {
        let error = render_icon([16, 33, 52], [164, 87, 52], size).unwrap_err();
        assert!(error.contains("unsupported icon size"), "{error}");
    }
}

#[test]
fn preserved_sources_keep_the_supplied_checksums() {
    let color = include_bytes!("../../../assets/icons/icon-color.svg");
    let mono = include_bytes!("../../../assets/icons/icon-mono.svg");

    assert_eq!(
        format!("{:x}", Sha256::digest(color)),
        "fa543f2ee4f1ab2acedc951e782c683f65b0b353db1deee368d0116fa62d8d08"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(mono)),
        "78eff50b0e91b1a6e9b53c73642f0fc13a2eaaa80f47a71dd73e789ab86968a5"
    );
}

#[test]
fn asset_builder_writes_status_app_ico_and_iconset_images() {
    let output = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_build-icons"))
        .args(["--out", output.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let ready_light = read_png(&output.path().join("status/light/ready-24.png"));
    let ready_dark = read_png(&output.path().join("status/dark/ready-24.png"));
    let warning_light = read_png(&output.path().join("status/light/warning-24.png"));
    let error_dark = read_png(&output.path().join("status/dark/error-24.png"));
    assert_eq!((ready_light.0, ready_light.1), (24, 24));
    assert!(
        ready_light
            .2
            .chunks_exact(4)
            .any(|p| p == [46, 173, 104, 255])
    );
    assert!(
        ready_light
            .2
            .chunks_exact(4)
            .any(|p| p == [16, 33, 52, 255])
    );
    assert!(
        ready_dark
            .2
            .chunks_exact(4)
            .any(|p| p == [240, 240, 240, 255])
    );
    assert!(
        warning_light
            .2
            .chunks_exact(4)
            .any(|p| p == [229, 167, 43, 255])
    );
    assert!(
        error_dark
            .2
            .chunks_exact(4)
            .any(|p| p == [217, 74, 72, 255])
    );
    assert_eq!(
        ready_light.2,
        render_icon([16, 33, 52], [46, 173, 104], 24).unwrap(),
        "PNG encoding must preserve straight-alpha edge pixels byte-for-byte"
    );
    assert_eq!(alpha_channel(&ready_light.2), alpha_channel(&ready_dark.2));
    assert_eq!(
        alpha_channel(&ready_light.2),
        alpha_channel(&warning_light.2)
    );
    assert_eq!(alpha_channel(&ready_dark.2), alpha_channel(&error_dark.2));

    for size in STATUS_SIZES {
        for appearance in ["light", "dark"] {
            for status in ["ready", "warning", "error"] {
                assert!(
                    output
                        .path()
                        .join(format!("status/{appearance}/{status}-{size}.png"))
                        .is_file()
                );
            }
        }
    }

    for size in APP_SIZES {
        let app = read_png(&output.path().join(format!("app/hieronymus-{size}.png")));
        assert_eq!((app.0, app.1), (size, size));
        assert!(app.2.chunks_exact(4).any(|p| p == [164, 87, 52, 255]));
        assert!(app.2.chunks_exact(4).any(|p| p == [16, 33, 52, 255]));
    }

    let icon_file = File::open(output.path().join("app/hieronymus.ico")).unwrap();
    let icon = ico::IconDir::read(icon_file).unwrap();
    let dimensions: Vec<_> = icon
        .entries()
        .iter()
        .map(|entry| (entry.width(), entry.height()))
        .collect();
    assert_eq!(
        dimensions,
        [
            (16, 16),
            (32, 32),
            (48, 48),
            (64, 64),
            (128, 128),
            (256, 256)
        ]
    );
    assert_eq!(
        icon.entries()[0].decode().unwrap().rgba_data(),
        render_icon([16, 33, 52], [164, 87, 52], 16).unwrap(),
        "ICO encoding must preserve straight-alpha edge pixels byte-for-byte"
    );

    for (name, size) in [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ] {
        let iconset = read_png(&output.path().join("app/hieronymus.iconset").join(name));
        assert_eq!((iconset.0, iconset.1), (size, size));
    }
}

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    rgba.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    (info.width, info.height, rgba)
}

fn alpha_channel(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).map(|pixel| pixel[3]).collect()
}
