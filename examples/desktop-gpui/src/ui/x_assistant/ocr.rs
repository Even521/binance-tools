use std::path::Path;

#[cfg(windows)]
pub(super) fn recognize_sent_toast(path: &Path) -> anyhow::Result<bool> {
    use windows::{
        Graphics::Imaging::BitmapDecoder,
        Media::Ocr::OcrEngine,
        Storage::StorageFile,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
        core::HSTRING,
    };

    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
    let _apartment = ApartmentGuard;
    let source = image::open(path)?;
    let enlarged = image::imageops::resize(
        &source,
        source.width().saturating_mul(4) / 3,
        source.height().saturating_mul(4) / 3,
        image::imageops::FilterType::Lanczos3,
    );
    let temp_path = std::env::temp_dir().join(format!(
        "x-assistant-ocr-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    enlarged.save(&temp_path)?;
    let recognition = (|| -> anyhow::Result<bool> {
        let path = temp_path.canonicalize()?;
        let path = path.to_string_lossy();
        let path = path.strip_prefix(r"\\?\").unwrap_or(path.as_ref());
        let path = HSTRING::from(path);
        let file = StorageFile::GetFileFromPathAsync(&path)?.get()?;
        let stream = file.OpenReadAsync()?.get()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
        let result = engine.RecognizeAsync(&bitmap)?.get()?;
        let text = result.Text()?.to_string();
        let normalized: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        Ok([
            "帖子已发送",
            "回复已发送",
            "yourpostwassent",
            "yourreplywassent",
        ]
        .iter()
        .any(|message| normalized.to_lowercase().contains(message)))
    })();
    let _ = std::fs::remove_file(temp_path);
    recognition
}

#[cfg(windows)]
struct ApartmentGuard;

#[cfg(windows)]
impl Drop for ApartmentGuard {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    }
}

#[cfg(not(windows))]
pub(super) fn recognize_sent_toast(_: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}
