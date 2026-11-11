//! Windows screen capture through GDI. Other platforms report it as unsupported.

use image::RgbImage;
use screenphoto::{Error, Result};
#[cfg(any(windows, test))]
use screenphoto::{LimitError, MAX_INPUT_PIXELS, checked_pixels};

#[cfg(windows)]
use std::sync::LazyLock;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{LPARAM, RECT};
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW,
    HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, ReleaseDC, SRCCOPY, SelectObject,
};
#[cfg(windows)]
use windows_sys::Win32::UI::HiDpi::{PROCESS_PER_MONITOR_DPI_AWARE, SetProcessDpiAwareness};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SetProcessDPIAware,
};
#[cfg(windows)]
use windows_sys::core::BOOL;

/// Grab a monitor (0-based) or the whole virtual desktop as RGB.
pub fn grab(monitor: Option<u32>) -> Result<RgbImage> {
    #[cfg(windows)]
    {
        windows_grab(monitor)
    }
    #[cfg(not(windows))]
    {
        let _ = monitor;
        Err(Error::Processing(
            "screen capture is only available on Windows with a visible desktop".into(),
        ))
    }
}

#[cfg(any(windows, test))]
fn checked_geometry(width: i32, height: i32) -> Result<(u32, u32)> {
    let (width, height) = (width as u32, height as u32);
    let pixels = checked_pixels(width, height).map_err(|error| match error {
        LimitError::Empty => Error::Processing(format!(
            "desktop reported a zero-sized rect ({width}x{height}); is the session locked?"
        )),
        LimitError::TooLarge => Error::Processing(format!(
            "screen capture {width}x{height} exceeds the {MAX_INPUT_PIXELS}-pixel input limit"
        )),
    })?;
    usize::try_from(pixels * 4)
        .map_err(|_| Error::Processing(format!("screen capture {width}x{height} is too large")))?;
    Ok((width, height))
}

#[cfg(windows)]
fn windows_grab(monitor: Option<u32>) -> Result<RgbImage> {
    make_dpi_aware();

    let (left, top, width, height) = match monitor {
        None => unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        },
        Some(index) => monitor_rect(index)?,
    };
    let (width, height) = checked_geometry(width, height)?;
    let (pixel_width, pixel_height) = (width as i32, height as i32);

    let screen = ScreenDc::open()?;
    let memory = MemoryDc::create(screen.handle())?;
    let bitmap = Bitmap::create(screen.handle(), pixel_width, pixel_height)?;
    {
        let _selected = Selection::new(memory.handle(), bitmap.handle())?;
        let copied = unsafe {
            BitBlt(
                memory.handle(),
                0,
                0,
                pixel_width,
                pixel_height,
                screen.handle(),
                left,
                top,
                SRCCOPY,
            )
        };
        if copied == 0 {
            return Err(os_error("BitBlt"));
        }
    }

    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: pixel_width,
            biHeight: -pixel_height, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buffer = vec![0u8; width as usize * height as usize * 4];
    let lines = unsafe {
        GetDIBits(
            memory.handle(),
            bitmap.handle(),
            0,
            height,
            buffer.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    if lines <= 0 {
        return Err(os_error("GetDIBits"));
    }
    if lines as u32 != height {
        return Err(Error::Processing(format!(
            "GetDIBits copied {lines} of {height} scan lines"
        )));
    }

    let pixel_count = width as usize * height as usize;
    for index in 0..pixel_count {
        let source = index * 4;
        let target = index * 3;
        let pixel = [buffer[source + 2], buffer[source + 1], buffer[source]];
        buffer[target..target + 3].copy_from_slice(&pixel);
    }
    buffer.truncate(pixel_count * 3);
    RgbImage::from_raw(width, height, buffer)
        .ok_or_else(|| Error::Processing("captured pixel buffer is inconsistent".into()))
}

#[cfg(windows)]
fn monitor_rect(index: u32) -> Result<(i32, i32, i32, i32)> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    let data = &mut monitors as *mut Vec<HMONITOR> as LPARAM;
    let enumerated = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            data,
        )
    };
    if enumerated == 0 {
        return Err(os_error("EnumDisplayMonitors"));
    }
    let Some(monitor) = monitors.get(index as usize).copied() else {
        return Err(Error::Processing(format!(
            "monitor {index} not found ({} present)",
            monitors.len()
        )));
    };

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return Err(Error::Processing(format!(
            "cannot query monitor {index}: {}",
            std::io::Error::last_os_error()
        )));
    }
    let rect = info.rcMonitor;
    rect_from_edges(rect.left, rect.top, rect.right, rect.bottom)
}

#[cfg(windows)]
unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    1
}

#[cfg(windows)]
fn rect_from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Result<(i32, i32, i32, i32)> {
    let width = i64::from(right) - i64::from(left);
    let height = i64::from(bottom) - i64::from(top);
    let width = i32::try_from(width)
        .map_err(|_| Error::Processing("desktop rect is too wide to capture".into()))?;
    let height = i32::try_from(height)
        .map_err(|_| Error::Processing("desktop rect is too tall to capture".into()))?;
    Ok((left, top, width, height))
}

#[cfg(windows)]
fn os_error(operation: &str) -> Error {
    Error::Processing(format!(
        "{operation} failed: {}",
        std::io::Error::last_os_error()
    ))
}

#[cfg(windows)]
fn make_dpi_aware() {
    static AWARENESS: LazyLock<()> = LazyLock::new(|| {
        // First call wins; failures mean the process is already DPI-aware.
        unsafe {
            let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
            let _ = SetProcessDPIAware();
        }
    });
    let _ = &*AWARENESS;
}

#[cfg(windows)]
struct ScreenDc(HDC);

#[cfg(windows)]
impl ScreenDc {
    fn open() -> Result<Self> {
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        if dc.is_null() {
            Err(os_error("GetDC"))
        } else {
            Ok(Self(dc))
        }
    }

    fn handle(&self) -> HDC {
        self.0
    }
}

#[cfg(windows)]
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), self.0);
        }
    }
}

#[cfg(windows)]
struct MemoryDc(HDC);

#[cfg(windows)]
impl MemoryDc {
    fn create(screen: HDC) -> Result<Self> {
        let dc = unsafe { CreateCompatibleDC(screen) };
        if dc.is_null() {
            Err(os_error("CreateCompatibleDC"))
        } else {
            Ok(Self(dc))
        }
    }

    fn handle(&self) -> HDC {
        self.0
    }
}

#[cfg(windows)]
impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.0);
        }
    }
}

#[cfg(windows)]
struct Bitmap(HBITMAP);

#[cfg(windows)]
impl Bitmap {
    fn create(screen: HDC, width: i32, height: i32) -> Result<Self> {
        let bitmap = unsafe { CreateCompatibleBitmap(screen, width, height) };
        if bitmap.is_null() {
            Err(os_error("CreateCompatibleBitmap"))
        } else {
            Ok(Self(bitmap))
        }
    }

    fn handle(&self) -> HBITMAP {
        self.0
    }
}

#[cfg(windows)]
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0);
        }
    }
}

/// A bitmap selected into a memory DC; dropping restores the old selection.
#[cfg(windows)]
struct Selection {
    dc: HDC,
    old: HGDIOBJ,
}

#[cfg(windows)]
impl Selection {
    fn new(dc: HDC, bitmap: HBITMAP) -> Result<Self> {
        let old = unsafe { SelectObject(dc, bitmap) };
        if old.is_null() || old as isize == -1 {
            return Err(os_error("SelectObject"));
        }
        Ok(Self { dc, old })
    }
}

#[cfg(windows)]
impl Drop for Selection {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::checked_geometry;

    #[test]
    fn geometry_rejects_empty_and_oversized_desktops() {
        assert!(checked_geometry(0, 10).is_err());
        assert!(checked_geometry(10, -1).is_err());
        assert!(checked_geometry(100_000, 100_000).is_err());
        assert_eq!(checked_geometry(64, 32).expect("valid"), (64, 32));
    }
}
