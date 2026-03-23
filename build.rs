use std::fs::File;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=logo.png");

    #[cfg(windows)]
    if let Err(error) = embed_windows_icon() {
        panic!("failed to embed Windows icon: {error}");
    }
}

#[cfg(windows)]
fn embed_windows_icon() -> Result<(), Box<dyn std::error::Error>> {
    use ico::{IconDir, IconImage, ResourceType};
    use image::imageops::FilterType;

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let logo_path = manifest_dir.join("logo.png");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let icon_path = out_dir.join("app-icon.ico");

    let source = image::open(&logo_path)?.into_rgba8();
    let mut icon_dir = IconDir::new(ResourceType::Icon);

    for size in [16, 24, 32, 48, 64, 128, 256] {
        let resized = image::imageops::resize(&source, size, size, FilterType::Lanczos3);
        let icon_image = IconImage::from_rgba_data(size, size, resized.into_raw());
        icon_dir.add_entry(ico::IconDirEntry::encode(&icon_image)?);
    }

    write_icon(&icon_path, icon_dir)?;

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_string_lossy().as_ref());
    resource.compile()?;

    Ok(())
}

#[cfg(windows)]
fn write_icon(path: &Path, icon_dir: ico::IconDir) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    icon_dir.write(file)?;
    Ok(())
}
