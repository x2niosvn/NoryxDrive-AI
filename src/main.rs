#![windows_subsystem = "windows"]

use noryxdrive_hud::config::{AppConfig, CropBox};
use noryxdrive_hud::ocr::{capture_screen_rect, run_ocr};
use noryxdrive_hud::solver::{SolveResult, SolverEngine};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW,
    CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
    FrameRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DRAW_TEXT_FORMAT,
    DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HBRUSH, HDC,
    HFONT, LOGFONTW, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, VK_F8, VK_F9, VK_XBUTTON1, VK_XBUTTON2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetSystemMetrics, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SendMessageW, SetLayeredWindowAttributes, ShowWindow, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, HICON, HTCAPTION, IDC_ARROW, IDC_CROSS, LWA_ALPHA, MA_NOACTIVATE,
    MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCLBUTTONDOWN, WM_PAINT, WM_USER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};

unsafe extern "system" {
    fn Beep(dw_freq: u32, dw_duration: u32) -> i32;
}

const WM_TRIGGER_SCAN: u32 = WM_USER + 1;
const WM_SCAN_FINISHED: u32 = WM_USER + 2;
const WM_OPEN_SELECTOR: u32 = WM_USER + 3;

const WINDOW_WIDTH: i32 = 360;
const WINDOW_HEIGHT: i32 = 270;

#[derive(Clone)]
enum UiState {
    Idle,
    Scanning,
    Solved(SolveResult),
    Error(String),
}

#[derive(Clone, Copy)]
struct SafeHwnd(isize);

impl SafeHwnd {
    fn from(h: HWND) -> Self {
        Self(h.0 as isize)
    }
    fn as_hwnd(&self) -> HWND {
        HWND(self.0 as *mut core::ffi::c_void)
    }
}

struct AppState {
    engine: Arc<SolverEngine>,
    config: Arc<Mutex<AppConfig>>,
    ui_state: Arc<Mutex<UiState>>,
    is_scanning: Arc<Mutex<bool>>,
    _hwnd: SafeHwnd,
}

static GLOBAL_STATE: OnceLock<AppState> = OnceLock::new();

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn create_font(height: i32, bold: bool) -> HFONT {
    unsafe {
        let mut lf: LOGFONTW = core::mem::zeroed();
        lf.lfHeight = -height;
        lf.lfWeight = if bold { 700 } else { 400 };
        lf.lfQuality = windows::Win32::Graphics::Gdi::FONT_QUALITY(5); // CLEARTYPE_QUALITY
        let font_name = to_wide("Segoe UI");
        for (i, &c) in font_name.iter().enumerate() {
            if i < lf.lfFaceName.len() {
                lf.lfFaceName[i] = c;
            }
        }
        CreateFontIndirectW(&lf)
    }
}

fn draw_text(hdc: HDC, text: &str, mut rect: RECT, flags: DRAW_TEXT_FORMAT, color: COLORREF) {
    unsafe {
        SetTextColor(hdc, color);
        let mut wide = to_wide(text);
        let _ = DrawTextW(hdc, &mut wide, &mut rect, flags);
    }
}

fn find_db_path() -> PathBuf {
    let candidate = PathBuf::from("noryxdrive.db");
    if candidate.exists() {
        return candidate;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate2 = parent.join("noryxdrive.db");
            if candidate2.exists() {
                return candidate2;
            }
        }
    }
    PathBuf::from("noryxdrive.db")
}

fn play_feedback_sound(is_critical: bool) {
    unsafe {
        if is_critical {
            Beep(1600, 120);
            thread::sleep(Duration::from_millis(50));
            Beep(1600, 120);
        } else {
            Beep(1200, 100);
        }
    }
}

fn execute_scan(
    engine: Arc<SolverEngine>,
    config: Arc<Mutex<AppConfig>>,
    ui_state: Arc<Mutex<UiState>>,
    is_scanning: Arc<Mutex<bool>>,
    hwnd: SafeHwnd,
) {
    let mut scanning_lock = is_scanning.lock().unwrap();
    if *scanning_lock {
        return;
    }
    *scanning_lock = true;
    drop(scanning_lock);

    {
        let mut state = ui_state.lock().unwrap();
        *state = UiState::Scanning;
    }
    unsafe {
        let _ = PostMessageW(Some(hwnd.as_hwnd()), WM_SCAN_FINISHED, WPARAM(0), LPARAM(0));
    }

    let is_scanning_clone = Arc::clone(&is_scanning);
    let ui_state_clone = Arc::clone(&ui_state);

    thread::spawn(move || {
        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

        let crop = {
            let cfg = config.lock().unwrap();
            cfg.crop_box.clone()
        };

        // Bước 1: Quét theo vùng crop nếu có
        let mut matched_result = None;

        if let Some(ref c) = crop {
            if let Ok(pixels) = capture_screen_rect(c.left, c.top, c.width, c.height) {
                if let Ok(text) = run_ocr(&pixels, c.width, c.height) {
                    if !text.trim().is_empty() {
                        let res = engine.solve(&text);
                        if res.matched {
                            matched_result = Some(res);
                        }
                    }
                }
            }
        }

        // Bước 2: Tự động dự phòng quét toàn màn hình nếu crop không khớp
        if matched_result.is_none() {
            if let Ok(pixels) = capture_screen_rect(0, 0, screen_w, screen_h) {
                if let Ok(text) = run_ocr(&pixels, screen_w, screen_h) {
                    if !text.trim().is_empty() {
                        let res = engine.solve(&text);
                        if res.matched {
                            matched_result = Some(res);
                        }
                    }
                }
            }
        }

        let new_state = if let Some(res) = matched_result {
            play_feedback_sound(res.is_critical);
            UiState::Solved(res)
        } else {
            UiState::Error("Không khớp câu hỏi nào trong bộ đề 600 câu".to_string())
        };

        {
            let mut state = ui_state_clone.lock().unwrap();
            *state = new_state;
        }

        {
            let mut scanning_lock = is_scanning_clone.lock().unwrap();
            *scanning_lock = false;
        }

        unsafe {
            let _ = PostMessageW(Some(hwnd.as_hwnd()), WM_SCAN_FINISHED, WPARAM(0), LPARAM(0));
        }
    });
}

// Bộ chọn vùng chụp tương tác (Region Selector)
static mut SEL_START_X: i32 = 0;
static mut SEL_START_Y: i32 = 0;
static mut SEL_CURR_X: i32 = 0;
static mut SEL_CURR_Y: i32 = 0;
static mut SEL_DRAGGING: bool = false;

unsafe extern "system" fn selector_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            SEL_START_X = (lparam.0 & 0xffff) as i16 as i32;
            SEL_START_Y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            SEL_CURR_X = SEL_START_X;
            SEL_CURR_Y = SEL_START_Y;
            SEL_DRAGGING = true;
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if SEL_DRAGGING {
                SEL_CURR_X = (lparam.0 & 0xffff) as i16 as i32;
                SEL_CURR_Y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if SEL_DRAGGING {
                SEL_DRAGGING = false;
                let end_x = (lparam.0 & 0xffff) as i16 as i32;
                let end_y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

                let left = SEL_START_X.min(end_x);
                let top = SEL_START_Y.min(end_y);
                let width = (SEL_START_X - end_x).abs();
                let height = (SEL_START_Y - end_y).abs();

                if width > 40 && height > 40 {
                    if let Some(state) = GLOBAL_STATE.get() {
                        let mut cfg = state.config.lock().unwrap();
                        cfg.crop_box = Some(CropBox {
                            left,
                            top,
                            width,
                            height,
                        });
                        cfg.save("hud_config.json");
                    }
                    Beep(1200, 100);
                }

                let _ = DestroyWindow(hwnd);
                if let Some(state) = GLOBAL_STATE.get() {
                    let _ = InvalidateRect(Some(state._hwnd.as_hwnd()), None, false);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let mem_dc = CreateCompatibleDC(Some(hdc));
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());

            // Nền tối mờ
            let bg_brush = CreateSolidBrush(rgb(10, 10, 10));
            FillRect(mem_dc, &rect, bg_brush);
            let _ = DeleteObject(bg_brush.into());

            if SEL_DRAGGING {
                let rx1 = SEL_START_X.min(SEL_CURR_X);
                let ry1 = SEL_START_Y.min(SEL_CURR_Y);
                let rx2 = SEL_START_X.max(SEL_CURR_X);
                let ry2 = SEL_START_Y.max(SEL_CURR_Y);

                let sel_rect = RECT {
                    left: rx1,
                    top: ry1,
                    right: rx2,
                    bottom: ry2,
                };
                let green_pen = CreatePen(PS_SOLID, 2, rgb(34, 197, 94));
                let old_pen = SelectObject(mem_dc, green_pen.into());
                let fill_brush = CreateSolidBrush(rgb(37, 99, 235));
                FillRect(mem_dc, &sel_rect, fill_brush);
                FrameRect(mem_dc, &sel_rect, CreateSolidBrush(rgb(34, 197, 94)));
                let _ = DeleteObject(fill_brush.into());
                SelectObject(mem_dc, old_pen);
                let _ = DeleteObject(green_pen.into());
            }

            let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);
            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn open_selector_window() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class_name = to_wide("NoryxDriveSelector");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(selector_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(instance.0),
            hIcon: HICON::default(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hIconSm: HICON::default(),
        };

        let _ = RegisterClassExW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(to_wide("Chon Vung").as_ptr()),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            screen_w,
            screen_h,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        ).unwrap();

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 90, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;

            // Nút đóng ✕ ở góc trên bên phải
            if x >= WINDOW_WIDTH - 32 && y <= 28 {
                PostQuitMessage(0);
                return LRESULT(0);
            }

            // Nút [ Chọn vùng ] ở thanh điều khiển (x: 10..120, y: 32..54)
            if x >= 10 && x <= 120 && y >= 32 && y <= 54 {
                open_selector_window();
                return LRESULT(0);
            }

            // Nút [ Toàn màn hình ] ở thanh điều khiển (x: 125..240, y: 32..54)
            if x >= 125 && x <= 240 && y >= 32 && y <= 54 {
                if let Some(state) = GLOBAL_STATE.get() {
                    let mut cfg = state.config.lock().unwrap();
                    cfg.crop_box = None;
                    cfg.save("hud_config.json");
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    Beep(1000, 80);
                }
                return LRESULT(0);
            }

            // Nút [ Quét ngay ] ở góc dưới bên phải
            if x >= WINDOW_WIDTH - 110 && y >= WINDOW_HEIGHT - 36 && y <= WINDOW_HEIGHT - 6 {
                if let Some(state) = GLOBAL_STATE.get() {
                    execute_scan(
                        Arc::clone(&state.engine),
                        Arc::clone(&state.config),
                        Arc::clone(&state.ui_state),
                        Arc::clone(&state.is_scanning),
                        SafeHwnd::from(hwnd),
                    );
                }
                return LRESULT(0);
            }

            // Kéo thả cửa sổ khi bấm vào thanh tiêu đề hoặc nền
            let _ = ReleaseCapture();
            let _ = SendMessageW(
                hwnd,
                WM_NCLBUTTONDOWN,
                Some(WPARAM(HTCAPTION as usize)),
                Some(LPARAM(0)),
            );
            LRESULT(0)
        }
        WM_TRIGGER_SCAN => {
            if let Some(state) = GLOBAL_STATE.get() {
                execute_scan(
                    Arc::clone(&state.engine),
                    Arc::clone(&state.config),
                    Arc::clone(&state.ui_state),
                    Arc::clone(&state.is_scanning),
                    SafeHwnd::from(hwnd),
                );
            }
            LRESULT(0)
        }
        WM_OPEN_SELECTOR => {
            open_selector_window();
            LRESULT(0)
        }
        WM_SCAN_FINISHED => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let mem_dc = CreateCompatibleDC(Some(hdc));
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());

            // 1. Nền tối Slate
            let bg_brush = CreateSolidBrush(rgb(15, 23, 42)); // #0f172a
            FillRect(mem_dc, &rect, bg_brush);
            let _ = DeleteObject(bg_brush.into());

            // 2. Viền ngoài xanh dương
            let border_brush = CreateSolidBrush(rgb(59, 130, 246)); // #3b82f6
            FrameRect(mem_dc, &rect, border_brush);
            let _ = DeleteObject(border_brush.into());

            // 3. Thanh tiêu đề Header
            let header_rect = RECT {
                left: 1,
                top: 1,
                right: width - 1,
                bottom: 28,
            };
            let header_brush = CreateSolidBrush(rgb(30, 41, 59)); // #1e293b
            FillRect(mem_dc, &header_rect, header_brush);
            let _ = DeleteObject(header_brush.into());

            SetBkMode(mem_dc, TRANSPARENT);

            let title_font = create_font(13, true);
            let _old_font = SelectObject(mem_dc, title_font.into());

            draw_text(
                mem_dc,
                "NoryxDrive AI - Nguyễn Nam X2NIOS",
                RECT {
                    left: 10,
                    top: 6,
                    right: width - 40,
                    bottom: 25,
                },
                DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
                rgb(248, 250, 252),
            );

            // Nút đóng ✕
            draw_text(
                mem_dc,
                "✕",
                RECT {
                    left: width - 28,
                    top: 5,
                    right: width - 6,
                    bottom: 25,
                },
                DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
                rgb(148, 163, 184),
            );

            // 4. Thanh chọn vùng / Toàn màn hình
            let ctrl_font = create_font(11, true);
            SelectObject(mem_dc, ctrl_font.into());

            let current_crop = if let Some(state) = GLOBAL_STATE.get() {
                state.config.lock().unwrap().crop_box.clone()
            } else {
                None
            };

            // Nút [ Chọn vùng (F9) ]
            let btn_crop_rect = RECT { left: 10, top: 32, right: 120, bottom: 54 };
            let is_custom_crop = current_crop.is_some();
            let crop_bg = if is_custom_crop { rgb(22, 163, 74) } else { rgb(51, 65, 85) };
            let crop_brush = CreateSolidBrush(crop_bg);
            FillRect(mem_dc, &btn_crop_rect, crop_brush);
            let _ = DeleteObject(crop_brush.into());

            let crop_text = if let Some(c) = current_crop {
                format!("Vùng: {}x{}", c.width, c.height)
            } else {
                "Chọn vùng (F9)".to_string()
            };
            draw_text(
                mem_dc,
                &crop_text,
                btn_crop_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                rgb(255, 255, 255),
            );

            // Nút [ Toàn màn hình ]
            let btn_full_rect = RECT { left: 125, top: 32, right: 240, bottom: 54 };
            let full_bg = if is_custom_crop { rgb(51, 65, 85) } else { rgb(37, 99, 235) };
            let full_brush = CreateSolidBrush(full_bg);
            FillRect(mem_dc, &btn_full_rect, full_brush);
            let _ = DeleteObject(full_brush.into());

            draw_text(
                mem_dc,
                "Toàn màn hình",
                btn_full_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                rgb(255, 255, 255),
            );

            let _ = DeleteObject(ctrl_font.into());

            // 5. Khung nội dung chính
            let content_rect = RECT {
                left: 10,
                top: 60,
                right: width - 10,
                bottom: height - 42,
            };
            let card_brush = CreateSolidBrush(rgb(30, 41, 59)); // #1e293b
            FillRect(mem_dc, &content_rect, card_brush);
            let _ = DeleteObject(card_brush.into());

            let card_border = CreateSolidBrush(rgb(51, 65, 85)); // #334155
            FrameRect(mem_dc, &content_rect, card_border);
            let _ = DeleteObject(card_border.into());

            let current_state = if let Some(state) = GLOBAL_STATE.get() {
                state.ui_state.lock().unwrap().clone()
            } else {
                UiState::Idle
            };

            match current_state {
                UiState::Idle => {
                    let idle_font = create_font(15, true);
                    SelectObject(mem_dc, idle_font.into());
                    draw_text(
                        mem_dc,
                        "ĐANG CHỜ QUÉT",
                        RECT {
                            left: 15,
                            top: 95,
                            right: width - 15,
                            bottom: 125,
                        },
                        DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
                        rgb(148, 163, 184),
                    );
                    let _ = DeleteObject(idle_font.into());

                    let sub_font = create_font(12, false);
                    SelectObject(mem_dc, sub_font.into());
                    draw_text(
                        mem_dc,
                        "Bấm Chuột phụ 2 (Mouse 5) hoặc F8",
                        RECT {
                            left: 15,
                            top: 130,
                            right: width - 15,
                            bottom: 160,
                        },
                        DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
                        rgb(100, 116, 139),
                    );
                    let _ = DeleteObject(sub_font.into());
                }
                UiState::Scanning => {
                    let scan_font = create_font(16, true);
                    SelectObject(mem_dc, scan_font.into());
                    draw_text(
                        mem_dc,
                        "ĐANG PHÂN TÍCH...",
                        RECT {
                            left: 15,
                            top: 110,
                            right: width - 15,
                            bottom: 140,
                        },
                        DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
                        rgb(56, 189, 248), // #38bdf8 cyan
                    );
                    let _ = DeleteObject(scan_font.into());
                }
                UiState::Solved(res) => {
                    let ans_font = create_font(24, true);
                    SelectObject(mem_dc, ans_font.into());
                    draw_text(
                        mem_dc,
                        &format!("ĐÁP ÁN: [ {} ]", res.correct_option),
                        RECT {
                            left: 15,
                            top: 65,
                            right: width - 15,
                            bottom: 96,
                        },
                        DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
                        rgb(34, 197, 94), // #22c55e Emerald
                    );
                    let _ = DeleteObject(ans_font.into());

                    let info_font = create_font(11, false);
                    SelectObject(mem_dc, info_font.into());
                    draw_text(
                        mem_dc,
                        &format!("Bộ đề 600 câu (Câu #{})", res.id),
                        RECT {
                            left: 16,
                            top: 98,
                            right: width - 16,
                            bottom: 116,
                        },
                        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
                        rgb(148, 163, 184),
                    );

                    let opt_font = create_font(12, true);
                    SelectObject(mem_dc, opt_font.into());
                    draw_text(
                        mem_dc,
                        &res.option_text,
                        RECT {
                            left: 16,
                            top: 118,
                            right: width - 16,
                            bottom: 172,
                        },
                        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                        rgb(241, 245, 249),
                    );
                    let _ = DeleteObject(opt_font.into());

                    let warn_font = create_font(11, true);
                    SelectObject(mem_dc, warn_font.into());

                    if res.is_critical {
                        draw_text(
                            mem_dc,
                            "CÂU ĐIỂM LIỆT (Sai là trượt ngay)",
                            RECT {
                                left: 16,
                                top: 176,
                                right: width - 16,
                                bottom: 196,
                            },
                            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
                            rgb(239, 68, 68), // Red
                        );
                    } else if res.is_trap {
                        let trap_str = if res.trap_reason.is_empty() {
                            "BẪY LỪA: Chú ý phương án!".to_string()
                        } else {
                            format!("BẪY LỪA: {}", res.trap_reason)
                        };
                        draw_text(
                            mem_dc,
                            &trap_str,
                            RECT {
                                left: 16,
                                top: 176,
                                right: width - 16,
                                bottom: 224,
                            },
                            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                            rgb(245, 158, 11), // Amber
                        );
                    } else if !res.explanation.is_empty() {
                        draw_text(
                            mem_dc,
                            &format!("Ghi chú: {}", res.explanation),
                            RECT {
                                left: 16,
                                top: 176,
                                right: width - 16,
                                bottom: 224,
                            },
                            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                            rgb(148, 163, 184),
                        );
                    }
                    let _ = DeleteObject(warn_font.into());
                    let _ = DeleteObject(info_font.into());
                }
                UiState::Error(ref err) => {
                    let err_font = create_font(13, true);
                    SelectObject(mem_dc, err_font.into());
                    draw_text(
                        mem_dc,
                        err,
                        RECT {
                            left: 16,
                            top: 105,
                            right: width - 16,
                            bottom: 170,
                        },
                        DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
                        rgb(239, 68, 68), // #ef4444 red
                    );
                    let _ = DeleteObject(err_font.into());
                }
            }

            // 6. Thanh đáy Footer
            let footer_font = create_font(11, false);
            SelectObject(mem_dc, footer_font.into());

            draw_text(
                mem_dc,
                "Phím: Mouse 5 | Mouse 4 | F8",
                RECT {
                    left: 12,
                    top: height - 30,
                    right: width - 120,
                    bottom: height - 8,
                },
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                rgb(100, 116, 139),
            );

            // Nút [ Quét ngay ]
            let btn_rect = RECT {
                left: width - 110,
                top: height - 34,
                right: width - 10,
                bottom: height - 8,
            };
            let btn_brush = CreateSolidBrush(rgb(37, 99, 235)); // #2563eb
            FillRect(mem_dc, &btn_rect, btn_brush);
            let _ = DeleteObject(btn_brush.into());

            let btn_font = create_font(11, true);
            SelectObject(mem_dc, btn_font.into());
            draw_text(
                mem_dc,
                "Quét ngay",
                btn_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                rgb(255, 255, 255),
            );

            let _ = DeleteObject(btn_font.into());
            let _ = DeleteObject(footer_font.into());
            let _ = DeleteObject(title_font.into());

            let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);

            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp.into());
            let _ = DeleteDC(mem_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    let db_path = find_db_path();
    let engine = match SolverEngine::new(&db_path) {
        Ok(eng) => Arc::new(eng),
        Err(e) => {
            eprintln!("Khong the nap co so du lieu: {:?}", e);
            return;
        }
    };

    let config_path = Path::new("hud_config.json");
    let config = Arc::new(Mutex::new(AppConfig::load(config_path)));
    let ui_state = Arc::new(Mutex::new(UiState::Idle));
    let is_scanning = Arc::new(Mutex::new(false));

    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class_name = to_wide("NoryxDriveFloatingHUD");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(instance.0),
            hIcon: HICON::default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hIconSm: HICON::default(),
        };

        RegisterClassExW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let start_x = screen_w - WINDOW_WIDTH - 25;
        let start_y = 65;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(to_wide("NoryxDrive AI - Nguyễn Nam X2NIOS").as_ptr()),
            WS_POPUP | WS_VISIBLE,
            start_x,
            start_y,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        ).unwrap();

        let _ = GLOBAL_STATE.set(AppState {
            engine,
            config,
            ui_state,
            is_scanning,
            _hwnd: SafeHwnd::from(hwnd),
        });

        let _ = ShowWindow(hwnd, SW_SHOW);

        // Luồng lắng nghe phím nóng toàn cục (Mouse 5, Mouse 4, F8, F9)
        let listener_hwnd = SafeHwnd::from(hwnd);
        thread::spawn(move || {
            let mut was_m5 = false;
            let mut was_m4 = false;
            let mut was_f8 = false;
            let mut was_f9 = false;

            loop {
                thread::sleep(Duration::from_millis(15));

                let is_m5 = (GetAsyncKeyState(VK_XBUTTON2.0 as i32) as u16 & 0x8000) != 0;
                let is_m4 = (GetAsyncKeyState(VK_XBUTTON1.0 as i32) as u16 & 0x8000) != 0;
                let is_f8 = (GetAsyncKeyState(VK_F8.0 as i32) as u16 & 0x8000) != 0;
                let is_f9 = (GetAsyncKeyState(VK_F9.0 as i32) as u16 & 0x8000) != 0;

                if (is_m5 && !was_m5) || (is_m4 && !was_m4) || (is_f8 && !was_f8) {
                    let _ = PostMessageW(Some(listener_hwnd.as_hwnd()), WM_TRIGGER_SCAN, WPARAM(0), LPARAM(0));
                }

                if is_f9 && !was_f9 {
                    let _ = PostMessageW(Some(listener_hwnd.as_hwnd()), WM_OPEN_SELECTOR, WPARAM(0), LPARAM(0));
                }

                was_m5 = is_m5;
                was_m4 = is_m4;
                was_f8 = is_f8;
                was_f9 = is_f9;
            }
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
