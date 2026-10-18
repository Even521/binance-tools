use std::{fs, io::Write, path::PathBuf};

pub(super) fn log_event(message: &str) {
    let path = PathBuf::from("data")
        .join("x_assistant")
        .join("mouse-click.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
    println!("[X辅助] {message}");
}

#[cfg(windows)]
pub(super) mod native {
    use std::{
        fs,
        mem::{size_of, zeroed},
        path::Path,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use windows::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap,
            CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SRCCOPY, SelectObject,
        },
        UI::{
            Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
                KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
                VIRTUAL_KEY,
            },
            WindowsAndMessaging::{
                FindWindowW, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
                SW_MAXIMIZE, SetCursorPos, SetForegroundWindow, ShowWindow,
            },
        },
    };

    pub fn activate_browser() -> anyhow::Result<()> {
        let foreground = unsafe { GetForegroundWindow() };
        let mut class_name = [0u16; 128];
        let count = if foreground.is_invalid() {
            0
        } else {
            unsafe { GetClassNameW(foreground, &mut class_name) }.max(0) as usize
        };
        let current_class = String::from_utf16_lossy(&class_name[..count]);
        let browser =
            if current_class == "Chrome_WidgetWin_1" || current_class == "MozillaWindowClass" {
                foreground
            } else {
                let chrome = unsafe { FindWindowW(windows::core::w!("Chrome_WidgetWin_1"), None) }
                    .unwrap_or_default();
                if chrome.is_invalid() {
                    unsafe { FindWindowW(windows::core::w!("MozillaWindowClass"), None) }
                        .unwrap_or_default()
                } else {
                    chrome
                }
            };
        anyhow::ensure!(
            !browser.is_invalid(),
            "未找到 Chrome、Edge 或 Firefox 窗口，请先打开 X 网页"
        );
        let _ = unsafe { ShowWindow(browser, SW_MAXIMIZE) };
        let _ = unsafe { SetForegroundWindow(browser) };
        thread::sleep(Duration::from_millis(350));
        anyhow::ensure!(
            unsafe { GetForegroundWindow() } == browser,
            "无法切换到浏览器，请手动将浏览器置于前台后重试"
        );
        Ok(())
    }

    pub fn move_cursor_to_reply(window_x: i32, window_y: i32) -> anyhow::Result<()> {
        let hwnd = unsafe { GetForegroundWindow() };
        anyhow::ensure!(!hwnd.is_invalid(), "无法找到前台浏览器窗口");
        let mut target = POINT {
            x: window_x,
            y: window_y,
        };
        anyhow::ensure!(
            unsafe { ClientToScreen(hwnd, &mut target) }.as_bool(),
            "无法将浏览器客户区坐标转换为屏幕坐标"
        );
        let target_x = target.x;
        let target_y = target.y;
        super::log_event(&format!(
            "鼠标目标：客户区坐标 ({window_x}, {window_y}) -> 屏幕坐标 ({target_x}, {target_y})"
        ));
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor)? };

        let mut random = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let steps = 12 + (random % 9) as usize;
        let distance_x = (target_x - cursor.x) as f64;
        let distance_y = (target_y - cursor.y) as f64;
        let distance = (distance_x * distance_x + distance_y * distance_y)
            .sqrt()
            .max(1.0);
        let curve = 4.0 + (random % 15) as f64;
        let perpendicular_x = -distance_y / distance;
        let perpendicular_y = distance_x / distance;

        for step in 1..=steps {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let progress = step as f64 / steps as f64;
            let eased = progress * progress * (3.0 - 2.0 * progress);
            let arc = (std::f64::consts::PI * progress).sin() * curve;
            let jitter = (random % 5) as f64 - 2.0;
            let (next_x, next_y) = if step == steps {
                (target_x, target_y)
            } else {
                (
                    cursor.x + (distance_x * eased + perpendicular_x * arc + jitter).round() as i32,
                    cursor.y + (distance_y * eased + perpendicular_y * arc + jitter).round() as i32,
                )
            };
            send_mouse_absolute(next_x, next_y)?;
            thread::sleep(Duration::from_millis(20 + random % 31));
        }
        thread::sleep(Duration::from_millis(180 + random % 180));
        Ok(())
    }

    fn send_mouse_absolute(x: i32, y: i32) -> anyhow::Result<()> {
        unsafe { SetCursorPos(x, y)? };
        Ok(())
    }

    pub fn type_text(text: &str) -> anyhow::Result<()> {
        for ch in text.chars() {
            if ch == '\r' || ch == '\n' {
                continue;
            }
            let code = u16::try_from(ch as u32).unwrap_or(b'?' as u16);
            let key = |up| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code,
                        dwFlags: if up {
                            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                        } else {
                            KEYEVENTF_UNICODE
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let sent = unsafe { SendInput(&[key(false), key(true)], size_of::<INPUT>() as i32) };
            anyhow::ensure!(sent == 2, "模拟键盘输入失败");
            thread::sleep(Duration::from_millis(28 + (code as u64 % 47)));
        }
        Ok(())
    }

    pub fn click_current_position() -> anyhow::Result<()> {
        let mouse_input = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [
            mouse_input(MOUSEEVENTF_LEFTDOWN),
            mouse_input(MOUSEEVENTF_LEFTUP),
        ];
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        super::log_event(&format!(
            "鼠标左键点击事件：{}（已发送 {}/2 个事件）",
            if sent == inputs.len() as u32 {
                "成功"
            } else {
                "失败"
            },
            sent
        ));
        anyhow::ensure!(
            sent == inputs.len() as u32,
            "回复按钮点击失败；请确认浏览器和本程序以相同权限运行"
        );
        Ok(())
    }

    pub fn click_at_client_position(client_x: i32, client_y: i32) -> anyhow::Result<()> {
        let hwnd = unsafe { GetForegroundWindow() };
        anyhow::ensure!(!hwnd.is_invalid(), "无法找到前台浏览器窗口");
        let mut class_name = [0u16; 128];
        let count = unsafe { GetClassNameW(hwnd, &mut class_name) }.max(0) as usize;
        let class_name = String::from_utf16_lossy(&class_name[..count]);
        anyhow::ensure!(
            class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass",
            "浏览器不在前台，已停止，避免点到其他窗口"
        );
        let mut target = POINT {
            x: client_x,
            y: client_y,
        };
        anyhow::ensure!(
            unsafe { ClientToScreen(hwnd, &mut target) }.as_bool(),
            "无法将浏览器客户区坐标转换为屏幕坐标"
        );
        let screen_x = target.x;
        let screen_y = target.y;
        super::log_event(&format!(
            "点击目标：客户区坐标 ({client_x}, {client_y}) -> 屏幕坐标 ({screen_x}, {screen_y})"
        ));
        send_mouse_absolute(screen_x, screen_y)?;
        thread::sleep(Duration::from_millis(180));
        click_current_position()
    }

    pub fn capture_window(path: &Path) -> anyhow::Result<()> {
        let hwnd = unsafe { GetForegroundWindow() };
        anyhow::ensure!(!hwnd.is_invalid(), "无法找到前台浏览器窗口");
        let mut class_name = [0u16; 128];
        let count = unsafe { GetClassNameW(hwnd, &mut class_name) }.max(0) as usize;
        let class_name = String::from_utf16_lossy(&class_name[..count]);
        anyhow::ensure!(
            class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass",
            "请将 Chrome、Edge 或 Firefox 浏览器置于前台后再操作"
        );
        let mut rect = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rect)? };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        anyhow::ensure!(width > 0 && height > 0, "浏览器客户区尺寸无效");
        let mut origin = POINT { x: 0, y: 0 };
        anyhow::ensure!(
            unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool(),
            "无法获取浏览器客户区的屏幕原点"
        );
        super::log_event(&format!(
            "截图浏览器客户区：{}x{}，屏幕原点 ({}, {})",
            width, height, origin.x, origin.y
        ));

        let screen = unsafe { GetDC(None) };
        anyhow::ensure!(!screen.is_invalid(), "无法访问桌面合成画面");
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        anyhow::ensure!(!memory.is_invalid(), "无法创建截图缓冲区");
        let bitmap = unsafe { CreateCompatibleBitmap(screen, width, height) };
        anyhow::ensure!(!bitmap.is_invalid(), "无法创建截图位图");
        let previous = unsafe { SelectObject(memory, bitmap.into()) };
        let copied = unsafe {
            BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                origin.x,
                origin.y,
                SRCCOPY,
            )
        };
        if copied.is_err() {
            unsafe {
                SelectObject(memory, previous);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
            }
            anyhow::bail!("截取浏览器客户区失败");
        }

        let mut info: BITMAPINFO = unsafe { zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        unsafe {
            SelectObject(memory, previous);
        }
        let rows = unsafe {
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        unsafe {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
        }
        anyhow::ensure!(rows == height, "读取浏览器截图像素失败");
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        save_bmp(path, width, height, &pixels)
    }

    fn save_bmp(path: &Path, width: i32, height: i32, pixels: &[u8]) -> anyhow::Result<()> {
        let data_size = u32::try_from(pixels.len())?;
        let mut bmp = Vec::with_capacity(pixels.len() + 54);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(data_size + 54).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&width.to_le_bytes());
        bmp.extend_from_slice(&(-height).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&data_size.to_le_bytes());
        bmp.extend_from_slice(&[0; 16]);
        bmp.extend_from_slice(pixels);
        fs::write(path, bmp)?;
        Ok(())
    }
}

#[cfg(not(windows))]
pub(super) mod native {
    use anyhow::bail;
    use std::path::Path;

    pub fn activate_browser() -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
    pub fn move_cursor_to_reply(_: i32, _: i32) -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
    pub fn click_current_position() -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
    pub fn click_at_client_position(_: i32, _: i32) -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
    pub fn type_text(_: &str) -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
    pub fn capture_window(_: &Path) -> anyhow::Result<()> {
        bail!("X 辅助桌面控制仅支持 Windows")
    }
}
