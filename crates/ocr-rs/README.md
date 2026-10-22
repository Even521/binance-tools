# ocr-rs

Workspace-local OCR facade backed by `ppocr-rs` and PP-OCRv6 Tiny. The OCR pipeline and application integration are Rust; recognition runs locally through ONNX Runtime. No Python or PaddlePaddle installation is required.

PP-OCRv6 Tiny recognizes Chinese and English (plus other supported languages). The first engine initialization downloads the detection model, recognition model, and character dictionary into the local `ppocr-rs` cache. Later runs use the cached files. The test/application therefore needs network access only for the first model download.

## Usage

```rust,no_run
use ocr_rs::OcrEngine;

let mut engine = OcrEngine::new()?;
let text = engine.recognize_path("data/screenshot.png")?;
println!("{text}");
# Ok::<(), anyhow::Error>(())
```

For text boxes and confidence scores, use `recognize_path_detailed`:

```rust,no_run
use ocr_rs::OcrEngine;

let mut engine = OcrEngine::new()?;
let result = engine.recognize_path_detailed("data/screenshot.png")?;
for block in result.text_blocks {
    println!("{} (score {:.3})", block.text, block.text_score);
}
# Ok::<(), anyhow::Error>(())
```

Create one engine and reuse it for multiple images; model initialization is expensive. `recognize_bytes` and `recognize_image` are also available.

## Try recognition with a test image

Set a local image path and run the test with output enabled. On first run, PP-OCRv6 Tiny models are downloaded and cached automatically. In PowerShell:

```powershell
$env:OCR_TEST_IMAGE = "data/screenshot.png"
cargo test -p ocr-rs recognize_configured_image_and_print_result -- --nocapture
```

The test prints pretty JSON split into four screenshot regions: `browser` (tabs, current page, bookmarks), `left_navigation`, `timeline` (navigation, composer, new-post notices, and posts), and `right_sidebar` (search and vertically ordered sections). Each post has consistent author, account, verification, timestamp, text, image, and engagement-metric fields; fields that cannot be recognized are `null`. Image pixels are inspected only to estimate media presence, not to interpret image contents. Unassigned center-column text is returned in `timeline.unclassified`. The test fails if no text is detected. The image can contain Chinese or mixed Chinese/English text.

## Runtime note

`ppocr-rs` exposes the OCR pipeline in Rust and uses ONNX Runtime for model inference. The crate's `test-binaries` feature supplies the Windows x86_64 runtime binary at build time. This is not a Python/PaddlePaddle dependency, but it does mean inference includes the ONNX Runtime native library. Release builds are recommended for practical OCR performance.
