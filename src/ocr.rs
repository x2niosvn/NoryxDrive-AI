use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Globalization::Language;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    SRCCOPY,
};
use windows::Win32::System::StationsAndDesktops::{
    OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

pub fn capture_screen_rect(x: i32, y: i32, width: i32, height: i32) -> Result<Vec<u8>, windows::core::Error> {
    unsafe {
        // Gắn luồng vào Desktop đang tương tác của người dùng
        let desk = OpenInputDesktop(
            DESKTOP_CONTROL_FLAGS(0),
            false,
            DESKTOP_ACCESS_FLAGS(0x01FF),
        );
        if let Ok(d) = desk {
            let _ = SetThreadDesktop(d);
        }

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let (cap_x, cap_y, cap_w, cap_h) = if width <= 0 || height <= 0 {
            (0, 0, screen_w, screen_h)
        } else {
            let cx = x.clamp(0, screen_w - 1);
            let cy = y.clamp(0, screen_h - 1);
            let cw = width.min(screen_w - cx);
            let ch = height.min(screen_h - cy);
            if cw <= 10 || ch <= 10 {
                (0, 0, screen_w, screen_h)
            } else {
                (cx, cy, cw, ch)
            }
        };

        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        let hbmp = CreateCompatibleBitmap(hdc_screen, cap_w, cap_h);
        let old_bmp = SelectObject(hdc_mem, hbmp.into());

        let _ = BitBlt(hdc_mem, 0, 0, cap_w, cap_h, Some(hdc_screen), cap_x, cap_y, SRCCOPY);

        let mut bmi: BITMAPINFO = core::mem::zeroed();
        bmi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = cap_w;
        bmi.bmiHeader.biHeight = -cap_h; // top-down DIB
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let size = (cap_w * cap_h * 4) as usize;
        let mut buffer = vec![0u8; size];
        let _ = GetDIBits(
            hdc_screen,
            hbmp,
            0,
            cap_h as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        Ok(buffer)
    }
}

pub fn run_ocr(pixels: &[u8], width: i32, height: i32) -> Result<String, windows::core::Error> {
    if pixels.is_empty() || width <= 0 || height <= 0 {
        return Ok(String::new());
    }

    let lang = Language::CreateLanguage(&windows::core::HSTRING::from("en-US"))?;
    let engine = if OcrEngine::IsLanguageSupported(&lang)? {
        OcrEngine::TryCreateFromLanguage(&lang)?
    } else {
        OcrEngine::TryCreateFromUserProfileLanguages()?
    };

    let writer = DataWriter::new()?;
    writer.WriteBytes(pixels)?;
    let buffer = writer.DetachBuffer()?;

    let sb = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)?;

    let async_op = engine.RecognizeAsync(&sb)?;
    let result = pollster::block_on(async_op)?;

    let text = result.Text()?.to_string();
    Ok(text)
}
