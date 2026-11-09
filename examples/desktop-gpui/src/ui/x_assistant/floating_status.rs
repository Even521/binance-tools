//! Windows browser-toolbar status overlay for X automation.
//!
//! The overlay is intentionally passive: it never takes focus or intercepts mouse input.
//! It follows the browser window used by the automation and disappears when the task ends.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub struct FloatingStatus {
    stop: Arc<AtomicBool>,
}

impl FloatingStatus {
    /// Show the application logo and a running indicator over the browser toolbar.
    pub fn show(task_label: &'static str, accent_color: (u8, u8, u8)) -> Self {
        let stop = Arc::new(AtomicBool::new(false));

        #[cfg(windows)]
        {
            let thread_stop = Arc::clone(&stop);
            std::thread::spawn(move || windows_overlay::run(thread_stop, task_label, accent_color));
        }

        #[cfg(not(windows))]
        let _ = (task_label, accent_color);

        Self { stop }
    }

    /// Hide the overlay. The native window is destroyed by its owning thread.
    pub fn hide(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for FloatingStatus {
    fn drop(&mut self) {
        self.hide();
    }
}

#[cfg(not(windows))]
mod windows_overlay {
    use std::sync::{Arc, atomic::AtomicBool};

    pub(super) fn run(_: Arc<AtomicBool>, _: &'static str, _: (u8, u8, u8)) {}
}

#[cfg(windows)]
mod windows_overlay {
    use std::{
        mem::zeroed,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, Ordering},
        },
        thread,
        time::Duration,
    };

    use windows::Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
            FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION, FONT_QUALITY, PAINTSTRUCT,
            SelectObject, SetBkMode, SetTextColor, TextOutW, TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
            DispatchMessageW, DrawIconEx, EnumWindows, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, LoadCursorW, MSG, PM_REMOVE,
            PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
            TranslateMessage, UnregisterClassW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
            DI_NORMAL, HICON, HWND_TOPMOST, IMAGE_FLAGS, LWA_COLORKEY, SW_HIDE,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, WNDCLASSW, WM_DESTROY,
            WM_ERASEBKGND, WM_PAINT, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    };
    use windows::core::w;

    const OVERLAY_WIDTH: i32 = 180;
    const OVERLAY_HEIGHT: i32 = 40;
    // 与浏览器右边缘留出更宽的间距，避免贴边遮挡扩展按钮。
    const RIGHT_INSET: i32 = 32;
    const COLOR_KEY: COLORREF = COLORREF(0x00FF00FF);

    static ICON_HANDLE: AtomicIsize = AtomicIsize::new(0);
    static TASK_KIND: AtomicU8 = AtomicU8::new(0);
    static ACCENT_COLOR: AtomicU32 = AtomicU32::new(0x00D77800);

    pub(super) fn run(stop: Arc<AtomicBool>, task_label: &'static str, accent_color: (u8, u8, u8)) {
        let task_kind = match task_label {
            "自动发帖" => 1,
            "测试发帖" => 2,
            _ => 3,
        };
        TASK_KIND.store(task_kind, Ordering::Release);
        let (red, green, blue) = accent_color;
        ACCENT_COLOR.store(
            red as u32 | ((green as u32) << 8) | ((blue as u32) << 16),
            Ordering::Release,
        );

        unsafe {
            let instance = match GetModuleHandleW(None) {
                Ok(module) => HINSTANCE(module.0),
                Err(error) => {
                    app_log::x_warn!("悬浮状态：获取程序模块句柄失败：{error}");
                    return;
                }
            };

            let class_name = w!("BinanceToolsXStatusOverlay");
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name,
                hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW)
                    .unwrap_or_default(),
                ..zeroed()
            };

            if RegisterClassW(&class) == 0 {
                app_log::x_warn!("悬浮状态：注册窗口类失败");
                return;
            }

            let icon = match load_embedded_icon() {
                Some(icon) => icon,
                None => {
                    app_log::x_warn!("悬浮状态：无法读取程序 Logo，悬浮状态未显示");
                    let _ = UnregisterClassW(class_name, Some(instance));
                    return;
                }
            };
            ICON_HANDLE.store(icon.0 as isize, Ordering::Release);

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                class_name,
                w!("Binance Tools X automation"),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                OVERLAY_WIDTH,
                OVERLAY_HEIGHT,
                None,
                None,
                Some(instance),
                None,
            );

            let Ok(hwnd) = hwnd else {
                app_log::x_warn!("悬浮状态：创建原生窗口失败");
                DestroyIcon(icon);
                ICON_HANDLE.store(0, Ordering::Release);
                let _ = UnregisterClassW(class_name, Some(instance));
                return;
            };

            let _ = SetLayeredWindowAttributes(hwnd, COLOR_KEY, 0, LWA_COLORKEY);
            let mut message = MSG::default();

            while !stop.load(Ordering::Acquire) {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    if message.message == WM_DESTROY {
                        stop.store(true, Ordering::Release);
                        break;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }

                if stop.load(Ordering::Acquire) {
                    break;
                }

                if let Some((x, y)) = browser_toolbar_position() {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        x,
                        y,
                        OVERLAY_WIDTH,
                        OVERLAY_HEIGHT,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }

                thread::sleep(Duration::from_millis(180));
            }

            let _ = DestroyWindow(hwnd);
            DestroyIcon(icon);
            ICON_HANDLE.store(0, Ordering::Release);
            let _ = UnregisterClassW(class_name, Some(instance));
        }
    }

    fn browser_toolbar_position() -> Option<(i32, i32)> {
        // 优先跟随当前前台浏览器；若短暂失去前台，则从可见 Chrome 窗口中
        // 选择面积最大的主窗口，避免 FindWindowW 随枚举顺序命中其他窗口。
        unsafe {
            // 始终选取面积最大的可见 Chrome 主窗口，不跟随临时弹窗或其他小窗口。
            let mut candidates: Vec<(HWND, i64)> = Vec::new();
            let _ = EnumWindows(
                Some(collect_browser_windows),
                LPARAM(&mut candidates as *mut Vec<(HWND, i64)> as isize),
            );
            let browser = candidates.into_iter().max_by_key(|(_, area)| *area)?.0;

            if browser.is_invalid() || IsIconic(browser).as_bool() {
                return None;
            }

            let mut rect = RECT::default();
            GetWindowRect(browser, &mut rect).ok()?;

            // 工具条固定在浏览器地址栏右侧，并向左留出更多空间。
            let x = rect.right - OVERLAY_WIDTH - RIGHT_INSET;
            let y = rect.top + 48;
            Some((x, y))
        }
    }

    unsafe extern "system" fn collect_browser_windows(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        if unsafe { IsWindowVisible(hwnd) }.as_bool() && !unsafe { IsIconic(hwnd) }.as_bool() {
            let mut process_id = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
            if process_id != 0 && unsafe { is_chrome_process(process_id) } {
                let mut rect = RECT::default();
                if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                    let width = (rect.right - rect.left).max(0) as i64;
                    let height = (rect.bottom - rect.top).max(0) as i64;
                    if width > 0 && height > 0 {
                        let windows = unsafe { &mut *(data.0 as *mut Vec<(HWND, i64)>) };
                        windows.push((hwnd, width * height));
                    }
                }
            }
        }
        windows::core::BOOL(1)
    }

    unsafe fn is_chrome_process(process_id: u32) -> bool {
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows::Win32::Foundation::CloseHandle;

        let Ok(process) = (unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
        }) else {
            return false;
        };
        let mut path = [0u16; 1024];
        let mut length = path.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_FORMAT(0),
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut length,
            )
        };
        let _ = unsafe { CloseHandle(process) };
        if result.is_err() {
            return false;
        }
        String::from_utf16_lossy(&path[..length as usize])
            .rsplit(['\\', '/'])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("chrome.exe"))
    }

    fn load_embedded_icon() -> Option<HICON> {
        let bytes = include_bytes!("../../../assets/app.ico");
        if bytes.len() < 22 {
            return None;
        }

        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        if count == 0 || bytes.len() < 6 + count * 16 {
            return None;
        }

        let entry = 6;
        let image_size = u32::from_le_bytes([
            bytes[entry + 8],
            bytes[entry + 9],
            bytes[entry + 10],
            bytes[entry + 11],
        ]) as usize;
        let image_offset = u32::from_le_bytes([
            bytes[entry + 12],
            bytes[entry + 13],
            bytes[entry + 14],
            bytes[entry + 15],
        ]) as usize;
        let image_end = image_offset.checked_add(image_size)?;
        let image = bytes.get(image_offset..image_end)?;

        // Creating a native icon transfers raw resource bytes to Win32.
        unsafe { CreateIconFromResourceEx(image, true, 0x0003_0000, 24, 24, IMAGE_FLAGS(0)).ok() }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Window-procedure callbacks are invoked by Win32; isolate all native drawing
        // calls in one explicit unsafe block for Rust 2024.
        unsafe {
            match message {
                WM_ERASEBKGND => LRESULT(1),
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut paint);
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: OVERLAY_WIDTH,
                        bottom: OVERLAY_HEIGHT,
                    };
                    // 悬浮窗背景直接使用 GPUI 当前主题的主色，和主界面保持一致。
                    let accent = COLORREF(ACCENT_COLOR.load(Ordering::Acquire));
                    let background = CreateSolidBrush(accent);
                    let _ = FillRect(dc, &rect, background);
                    let _ = DeleteObject(background.into());

                    let icon = HICON(ICON_HANDLE.load(Ordering::Acquire) as _);
                    if !icon.is_invalid() {
                        let _ = DrawIconEx(dc, 8, 7, icon, 26, 26, 0, None, DI_NORMAL);
                    }

                    let _ = SetBkMode(dc, TRANSPARENT);
                    // 原生默认字体在 180px 宽的悬浮条里过大，显式缩小并使用 Segoe UI。
                    let title_font = CreateFontW(
                        -13, 0, 0, 0, 600, 0, 0, 0, FONT_CHARSET(1), FONT_OUTPUT_PRECISION(0),
                        FONT_CLIP_PRECISION(0), FONT_QUALITY(5), 0,
                        w!("Segoe UI"),
                    );
                    let task_font = CreateFontW(
                        -11, 0, 0, 0, 400, 0, 0, 0, FONT_CHARSET(1), FONT_OUTPUT_PRECISION(0),
                        FONT_CLIP_PRECISION(0), FONT_QUALITY(5), 0,
                        w!("Segoe UI"),
                    );
                    let title: Vec<u16> = "自动化运行中".encode_utf16().collect();
                    let task_label = match TASK_KIND.load(Ordering::Acquire) {
                        1 => "当前任务：自动发帖",
                        2 => "当前任务：测试发帖",
                        _ => "当前任务：自动回复",
                    };
                    let task: Vec<u16> = task_label.encode_utf16().collect();
                    // 根据主题背景亮度自动选择文字颜色，确保深色/浅色主题都清晰。
                    let red = (accent.0 & 0xFF) as f64;
                    let green = ((accent.0 >> 8) & 0xFF) as f64;
                    let blue = ((accent.0 >> 16) & 0xFF) as f64;
                    let luminance = 0.299 * red + 0.587 * green + 0.114 * blue;
                    let foreground = if luminance > 150.0 {
                        COLORREF(0x00181818)
                    } else {
                        COLORREF(0x00FFFFFF)
                    };
                    let _ = SelectObject(dc, title_font.into());
                    let _ = SetTextColor(dc, foreground);
                    let _ = TextOutW(dc, 44, 3, &title);
                    let _ = SelectObject(dc, task_font.into());
                    let _ = SetTextColor(dc, foreground);
                    let _ = TextOutW(dc, 44, 21, &task);
                    let _ = DeleteObject(title_font.into());
                    let _ = DeleteObject(task_font.into());

                    // 绿色状态圆点表示自动化任务正在运行。
                    let dot = CreateSolidBrush(COLORREF(0x0000C000));
                    let previous = SelectObject(dc, dot.into());
                    let _ = Ellipse(dc, 163, 7, 173, 17);
                    let _ = SelectObject(dc, previous);
                    let _ = DeleteObject(dot.into());

                    let _ = EndPaint(hwnd, &paint);
                    LRESULT(0)
                }
                WM_DESTROY => LRESULT(0),
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
    }
}
