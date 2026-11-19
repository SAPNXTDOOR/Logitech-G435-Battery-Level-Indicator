fn main() {
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        windows_icon::embed_icon();
    }
}

#[cfg(windows)]
mod windows_icon {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::{env, mem, ptr};

    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
        FF_DONTCARE, FW_NORMAL, GetDC, OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkMode,
        SetTextColor, TRANSPARENT,
    };

    // Set to false for a transparent background (white headphone glyph only).
    // true adds a dark rounded square so the icon stays visible in Windows Light Mode too.
    const DARK_BADGE: bool = true;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn in_rounded_rect(x: i32, y: i32, l: i32, t: i32, r: i32, b: i32, rad: i32) -> bool {
        if x < l || x >= r || y < t || y >= b {
            return false;
        }
        let dx = if x < l + rad {
            (l + rad) - x
        } else if x >= r - rad {
            x - (r - rad - 1)
        } else {
            0
        };
        let dy = if y < t + rad {
            (t + rad) - y
        } else if y >= b - rad {
            y - (b - rad - 1)
        } else {
            0
        };
        dx * dx + dy * dy <= rad * rad
    }

    fn render_dib(size: i32) -> Vec<u8> {
        let mut cov_map = vec![0u8; (size * size) as usize];

        unsafe {
            let screen_dc = GetDC(ptr::null_mut());
            let mem_dc = CreateCompatibleDC(screen_dc);

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..mem::zeroed()
                },
                bmiColors: [mem::zeroed()],
            };

            let mut bits: *mut core::ffi::c_void = ptr::null_mut();
            let hbm = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if !hbm.is_null() && !bits.is_null() {
                let old_bm = SelectObject(mem_dc, hbm as _);
                let pixels =
                    std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
                pixels.fill(0);

                SetBkMode(mem_dc, TRANSPARENT as i32);
                SetTextColor(mem_dc, 0x00FF_FFFF);

                let font_px = if DARK_BADGE {
                    (size * 65) / 100
                } else {
                    (size * 88) / 100
                };
                let font_name = wide("Segoe Fluent Icons");
                let font = CreateFontW(
                    -font_px,
                    0,
                    0,
                    0,
                    FW_NORMAL as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    ANTIALIASED_QUALITY as u32,
                    (DEFAULT_PITCH | FF_DONTCARE) as u32,
                    font_name.as_ptr(),
                );
                let old_font = SelectObject(mem_dc, font as _);
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: size,
                    bottom: size,
                };
                let glyph = wide("\u{E7F6}");
                DrawTextW(
                    mem_dc,
                    glyph.as_ptr(),
                    -1,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );

                for (i, &px) in pixels.iter().enumerate() {
                    cov_map[i] = ((px >> 16) & 0xFF) as u8;
                }

                SelectObject(mem_dc, old_font);
                DeleteObject(font as _);
                SelectObject(mem_dc, old_bm);
                DeleteObject(hbm as _);
            }

            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
        }

        let and_stride = ((size + 31) / 32) * 4;
        let xor_size = (size * size * 4) as usize;
        let and_size = (and_stride * size) as usize;
        let mut dib = Vec::with_capacity(40 + xor_size + and_size);

        // BITMAPINFOHEADER (biHeight must be 2 * size for ICO format)
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&size.to_le_bytes());
        dib.extend_from_slice(&(size * 2).to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&(xor_size as u32).to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());

        let mut and_mask = vec![0u8; and_size];
        let rad = (size / 5).max(2);
        let border = (size / 32).max(1);

        // Bottom-up BGRA pixels
        for (row_idx, y) in (0..size).rev().enumerate() {
            for x in 0..size {
                let cov = cov_map[(y * size + x) as usize] as u32;
                let (r, g, b, a): (u8, u8, u8, u8) = if DARK_BADGE {
                    if in_rounded_rect(x, y, 0, 0, size, size, rad) {
                        let inner = in_rounded_rect(
                            x,
                            y,
                            border,
                            border,
                            size - border,
                            size - border,
                            (rad - border).max(1),
                        );
                        let (br, bg, bb) = if inner { (32, 36, 42) } else { (62, 66, 74) };
                        let inv = 255 - cov;
                        (
                            ((255 * cov + br * inv) / 255) as u8,
                            ((255 * cov + bg * inv) / 255) as u8,
                            ((255 * cov + bb * inv) / 255) as u8,
                            255,
                        )
                    } else {
                        (0, 0, 0, 0)
                    }
                } else if cov > 0 {
                    (255, 255, 255, cov as u8)
                } else {
                    (0, 0, 0, 0)
                };

                dib.extend_from_slice(&[b, g, r, a]);

                if a == 0 {
                    let byte_idx = row_idx * and_stride as usize + (x / 8) as usize;
                    and_mask[byte_idx] |= 0x80 >> (x & 7);
                }
            }
        }

        dib.extend_from_slice(&and_mask);
        dib
    }

    fn write_ico(path: &Path) {
        let sizes = [16, 32, 48, 256];
        let images: Vec<(i32, Vec<u8>)> = sizes.iter().map(|&s| (s, render_dib(s))).collect();

        let mut ico = Vec::new();
        // ICONDIR header
        ico.extend_from_slice(&0u16.to_le_bytes());
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&(images.len() as u16).to_le_bytes());

        // ICONDIRENTRY table
        let mut offset = 6 + (images.len() * 16) as u32;
        for &(size, ref data) in &images {
            let dim = if size >= 256 { 0u8 } else { size as u8 };
            ico.push(dim);
            ico.push(dim);
            ico.push(0);
            ico.push(0);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += data.len() as u32;
        }

        for (_, data) in images {
            ico.extend_from_slice(&data);
        }

        fs::write(path, ico).expect("failed to write generated .ico");
    }

    pub fn embed_icon() {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let ico_path = out_dir.join("app.ico");
        write_ico(&ico_path);

        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("valid utf-8 path"));
        res.compile().expect("failed to embed Windows exe icon");
    }
}