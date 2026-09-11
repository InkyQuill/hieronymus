use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use hiero_desktop::render_icon;

const STATUS_SIZES: [u32; 9] = [16, 20, 22, 24, 32, 40, 44, 48, 64];
const APP_SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];
const ICO_SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

const LIGHT_FOREGROUND: [u8; 3] = [16, 33, 52];
const DARK_FOREGROUND: [u8; 3] = [240, 240, 240];
const BRAND_ACCENT: [u8; 3] = [164, 87, 52];

const STATUSES: [(&str, [u8; 3]); 3] = [
    ("ready", [46, 173, 104]),
    ("warning", [229, 167, 43]),
    ("error", [217, 74, 72]),
];
const APPEARANCES: [(&str, [u8; 3]); 2] = [("light", LIGHT_FOREGROUND), ("dark", DARK_FOREGROUND)];

fn main() {
    if let Err(error) = run(env::args_os().skip(1).collect()) {
        eprintln!("build-icons: {error}");
        std::process::exit(2);
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    let [flag, output]: [OsString; 2] = args.try_into().map_err(|_| usage())?;
    if flag != "--out" {
        return Err(usage());
    }
    let output = PathBuf::from(output);
    build_status_icons(&output)?;
    build_app_icons(&output)?;
    Ok(())
}

fn usage() -> String {
    "usage: build-icons --out <directory>".to_owned()
}

fn build_status_icons(output: &Path) -> Result<(), String> {
    for (appearance, foreground) in APPEARANCES {
        for (status, accent) in STATUSES {
            for size in STATUS_SIZES {
                let rgba = render_icon(foreground, accent, size)?;
                write_png(
                    &output.join(format!("status/{appearance}/{status}-{size}.png")),
                    size,
                    &rgba,
                )?;
            }
        }
    }
    Ok(())
}

fn build_app_icons(output: &Path) -> Result<(), String> {
    let app_dir = output.join("app");
    for size in APP_SIZES {
        let rgba = render_icon(LIGHT_FOREGROUND, BRAND_ACCENT, size)?;
        write_png(&app_dir.join(format!("hieronymus-{size}.png")), size, &rgba)?;
    }

    write_ico(&app_dir.join("hieronymus.ico"))?;
    let iconset = app_dir.join("hieronymus.iconset");
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
        let rgba = render_icon(LIGHT_FOREGROUND, BRAND_ACCENT, size)?;
        write_png(&iconset.join(name), size, &rgba)?;
    }
    Ok(())
}

fn write_png(path: &Path, size: u32, rgba: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("output has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let file = File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("could not encode {}: {error}", path.display()))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| format!("could not encode {}: {error}", path.display()))
}

fn write_ico(path: &Path) -> Result<(), String> {
    let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICO_SIZES {
        let rgba = render_icon(LIGHT_FOREGROUND, BRAND_ACCENT, size)?;
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        let entry = ico::IconDirEntry::encode(&image)
            .map_err(|error| format!("could not encode ICO entry {size}: {error}"))?;
        directory.add_entry(entry);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut file = BufWriter::new(
        File::create(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?,
    );
    directory
        .write(&mut file)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("could not flush {}: {error}", path.display()))
}
