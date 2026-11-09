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
    pub fn show() -> Self {
        let stop = Arc::new(AtomicBool::new(false));

        #[cfg(windows)]
        {
            let thread_stop = Arc::clone(&stop);
            std::thread::spawn(move || windows_overlay::run(thread_stop));
        }

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

    pub(super) fn run(_: Arc<AtomicBool>) {}
}

#[cfg(windows)]
mod windows_overlay {
    use std::{
        mem::zeroed,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicIsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use windows::Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, COLORREF, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
            PAINTSTRUCT, SelectObject,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
            DispatchMessageW, DrawIconEx, FindWindowW, GetWindowRect, IsIconic, LoadCursorW,
            MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos,
            ShowWindow, TranslateMessage, UnregisterClassW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
            DI_NORMAL, HICON, HWND_TOPMOST, IMAGE_FLAGS, LWA_COLORKEY, SW_HIDE,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, WNDCLASSW, WM_DESTROY,
            WM_ERASEBKGND, WM_PAINT, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    };
    use windows::core::w;

    const OVERLAY_WIDTH: i32 = 42;
    const OVERLAY_HEIGHT: i32 = 28;
    const COLOR_KEY: COLORREF = COLORREF(0x00FF00FF);

    static ICON_HANDLE: AtomicIsize = AtomicIsize::new(0);

    pub(super) fn run(stop: Arc<AtomicBool>) {
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

    unsafe fn browser_toolbar_position() -> Option<(i32, i32)> {
        let browser = FindWindowW(w!("Chrome_WidgetWin_1"), None)
            .or_else(|_| FindWindowW(w!("MozillaWindowClass"), None))
            .ok()?;

        if browser.is_invalid() || IsIconic(browser).as_bool() {
            return None;
        }

        let mut rect = RECT::default();
        GetWindowRect(browser, &mut rect).ok()?;

        // Chromium's address/toolbar row is near the top of the non-client area.
        // Keep the small logo marker at the far-right end of that row.
        let x = rect.right - OVERLAY_WIDTH - 12;
        let y = rect.top + 48;
        Some((x, y))
    }

    unsafe fn load_embedded_icon() -> Option<HICON> {
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

        CreateIconFromResourceEx(image, true, 0x0003_0000, 24, 24, IMAGE_FLAGS(0))
        .ok()
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
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
                let background = CreateSolidBrush(COLOR_KEY);
                let _ = FillRect(dc, &rect, background);
                let _ = DeleteObject(background.into());

                let icon = HICON(ICON_HANDLE.load(Ordering::Acquire) as _);
                if !icon.is_invalid() {
                    let _ = DrawIconEx(dc, 1, 2, icon, 24, 24, 0, None, DI_NORMAL);
                }

                // Green dot means the automation task is active.
                let dot = CreateSolidBrush(COLORREF(0x0000C853));
                let previous = SelectObject(dc, dot.into());
                let _ = Ellipse(dc, 30, 9, 40, 19);
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
