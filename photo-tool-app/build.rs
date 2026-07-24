fn main() {
    #[cfg(target_os = "windows")]
    {
        use std::path::Path;

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let manifest_dir = Path::new(&manifest_dir);
        let png_path = manifest_dir.join("src/assets/icon.png");
        let ico_path = manifest_dir.join("src/assets/icon.ico");
        let rc_path = manifest_dir.join("src/assets/app.rc");

        if !ico_path.exists() || is_newer(&png_path, &ico_path) {
            generate_ico(&png_path, &ico_path);
        }

        std::fs::write(&rc_path, "MAINICON ICON \"icon.ico\"\n")
            .expect("Failed to write app.rc");
        embed_resource::compile("src/assets/app.rc", &[] as &[&str]);
    }
}

#[cfg(target_os = "windows")]
fn is_newer(a: &std::path::Path, b: &std::path::Path) -> bool {
    let am = std::fs::metadata(a).and_then(|m| m.modified());
    let bm = std::fs::metadata(b).and_then(|m| m.modified());
    matches!((am, bm), (Ok(at), Ok(bt)) if at > bt)
}

#[cfg(target_os = "windows")]
fn generate_ico(png_path: &std::path::Path, ico_path: &std::path::Path) {
    let img = image::open(png_path).expect("Failed to load icon.png");
    let resized = img.resize_exact(32, 32, image::imageops::Lanczos3);
    let rgba = resized.to_rgba8();
    let raw = rgba.as_raw();

    // BMP DIB (BITMAPINFOHEADER + BGRA pixels, bottom-up)
    let w = 32u32;
    let h = 32u32;
    let row_size = w as usize * 4; // 32bpp = 4 bytes per pixel
    let data_size = row_size * h as usize;
    let header_size = 40u32;
    let dib_size = header_size + data_size as u32;

    let mut dib = Vec::with_capacity(dib_size as usize);

    // BITMAPINFOHEADER (40 bytes)
    dib.extend_from_slice(&header_size.to_le_bytes());
    dib.extend_from_slice(&w.to_le_bytes());
    dib.extend_from_slice(&(h * 2).to_le_bytes()); // height*2 = top-down + AND mask
    dib.extend_from_slice(&1u16.to_le_bytes()); // planes
    dib.extend_from_slice(&32u16.to_le_bytes()); // bpp
    dib.extend_from_slice(&0u32.to_le_bytes()); // compression = BI_RGB
    dib.extend_from_slice(&data_size.to_le_bytes()); // size image
    dib.extend_from_slice(&0u32.to_le_bytes()); // x pixels per meter
    dib.extend_from_slice(&0u32.to_le_bytes()); // y pixels per meter
    dib.extend_from_slice(&0u32.to_le_bytes()); // colors used
    dib.extend_from_slice(&0u32.to_le_bytes()); // important colors

    // BGRA pixels (bottom-up) — ICO uses BGRA format in DIB
    for y in (0..h).rev() {
        let row_start = (y * w) as usize * 4;
        for x in 0..w as usize {
            let px = row_start + x * 4;
            dib.push(raw[px + 2]); // B
            dib.push(raw[px + 1]); // G
            dib.push(raw[px]);     // R
            dib.push(raw[px + 3]); // A
        }
    }
    // AND mask: 1bpp, row padded to 4 bytes. All zeros = fully opaque
    let and_row_size = ((w + 31) / 32) as usize * 4;
    for _ in 0..h {
        dib.extend(std::iter::repeat(0u8).take(and_row_size));
    }

    // ico file: header + directory + DIB
    let mut ico = Vec::new();
    // Header
    ico.extend_from_slice(&[0u8, 0, 1, 0, 1, 0]);
    // Directory entry
    ico.push(32);
    ico.push(32);
    ico.push(0);
    ico.push(0);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // offset = 6 + 16
    // DIB data
    ico.append(&mut dib);

    std::fs::write(ico_path, &ico).expect("Failed to write icon.ico");
}
