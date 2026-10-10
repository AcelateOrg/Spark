//! Executable icons for `spark build`, made from game.toml `icon` (PNG / JPEG, ideally square, 256+ px):
//! Windows - an icon resource inside the .exe (Explorer, desktop shortcuts, taskbar pins),
//! macOS - `Contents/Resources/icon.icns` in the `.app` bundle. The window icon itself is set at runtime.

use std::path::Path;

use image::{ImageEncoder, RgbaImage};

/// Loads the icon and pads it to a square (transparent borders).
pub fn load(path: &Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("cannot load '{}': {e}", path.display()))?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == h {
        return Ok(img);
    }
    let s = w.max(h);
    let mut sq = RgbaImage::new(s, s);
    image::imageops::overlay(&mut sq, &img, ((s - w) / 2) as i64, ((s - h) / 2) as i64);
    Ok(sq)
}

/// PNG bytes of the icon scaled to `size` x `size`.
fn png_at(img: &RgbaImage, size: u32) -> Result<Vec<u8>, String> {
    let scaled = if img.width() == size { img.clone() } else { image::imageops::resize(img, size, size, image::imageops::FilterType::Lanczos3) };
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(scaled.as_raw(), size, size, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("cannot encode the icon: {e}"))?;
    Ok(out)
}

/// macOS `.icns` (PNG entries 128..1024 px).
pub fn icns(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    for (tag, size) in [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)] {
        let png = png_at(img, size)?;
        body.extend_from_slice(tag);
        body.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Sizes inside the Windows icon (PNG-compressed entries, Vista+).
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Returns `exe` (a PE file without a pack) with the icon as its main icon resource.
#[cfg(windows)]
pub fn set_exe_icon(exe: &[u8], img: &RgbaImage) -> Result<Vec<u8>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW};

    const RT_ICON: usize = 3;
    const RT_GROUP_ICON: usize = 14;
    let pngs = ICO_SIZES.iter().map(|&s| png_at(img, s)).collect::<Result<Vec<_>, _>>()?;
    // GRPICONDIR + GRPICONDIRENTRY per image (ids 1..n)
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
    for (i, (png, &size)) in pngs.iter().zip(ICO_SIZES.iter()).enumerate() {
        let px = if size >= 256 { 0u8 } else { size as u8 };
        group.extend_from_slice(&[px, px, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(png.len() as u32).to_le_bytes());
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }

    let tmp = std::env::temp_dir().join(format!("spark-build-icon-{}.exe", std::process::id()));
    std::fs::write(&tmp, exe).map_err(|e| format!("cannot write '{}': {e}", tmp.display()))?;
    let wide: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        let h = BeginUpdateResourceW(wide.as_ptr(), 0);
        if h.is_null() {
            Err("BeginUpdateResource failed".to_string())
        } else {
            let mut ok = true;
            for (i, png) in pngs.iter().enumerate() {
                ok &= UpdateResourceW(h, RT_ICON as *const u16, (i + 1) as *const u16, 0, png.as_ptr().cast(), png.len() as u32) != 0;
            }
            ok &= UpdateResourceW(h, RT_GROUP_ICON as *const u16, 1usize as *const u16, 0, group.as_ptr().cast(), group.len() as u32) != 0;
            if !ok {
                EndUpdateResourceW(h, 1);
                Err("UpdateResource failed".to_string())
            } else if EndUpdateResourceW(h, 0) == 0 {
                Err("EndUpdateResource failed".to_string())
            } else {
                Ok(())
            }
        }
    };
    let out = result.and_then(|()| std::fs::read(&tmp).map_err(|e| format!("cannot read '{}': {e}", tmp.display())));
    let _ = std::fs::remove_file(&tmp);
    out
}

#[cfg(not(windows))]
pub fn set_exe_icon(exe: &[u8], _img: &RgbaImage) -> Result<Vec<u8>, String> {
    Ok(exe.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icns_layout() {
        let img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 0, 0, 255]));
        let d = icns(&img).unwrap();
        assert_eq!(&d[0..4], b"icns");
        assert_eq!(u32::from_be_bytes(d[4..8].try_into().unwrap()) as usize, d.len());
        assert_eq!(&d[8..12], b"ic07");
    }
}
