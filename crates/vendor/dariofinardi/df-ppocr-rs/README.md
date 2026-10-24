# ppocr-rs

> Pure-Rust OCR pipeline — PP-OCRv6 (50 languages) + PP-DocLayoutV3 +
> RT-DETR-L Table Cell Detection + SLANet_plus Table Structure +
> PP-LCNet Doc Orientation, on `ort 2.0.0-rc.13` (ONNX Runtime).

[![crates.io](https://img.shields.io/crates/v/ppocr-rs.svg)](https://crates.io/crates/ppocr-rs)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Status: Beta](https://img.shields.io/badge/Status-Beta-blue.svg)](#)
[![Rust 2021](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)

Crate name: **`ppocr-rs`**. Maintained by [dariofinardi](https://github.com/dariofinardi)
under Apache-2.0.

---

## Installation

```toml
[dependencies]
ppocr-rs = "0.8.4"
```

**ONNX Runtime 1.28 is required.** The crate does not link the runtime
statically: on first use, `ort` looks for `onnxruntime.dll` /
`libonnxruntime.so` next to your binary. An older library fails with
`BadVersion` when the first model is loaded — see [Build](#build) for the
two ways to provide it (automatic download on x86_64, `load-dynamic` on
ARM64).

No other system dependency: no OpenCV, no Python, no PaddlePaddle.

---

## Overview

A complete document OCR pipeline in pure Rust, **with no system
dependencies**. The whole runtime is in-process through
[`ort`](https://crates.io/crates/ort) (ONNX Runtime bindings). It is the
OCR backend of Omissis.Edge, a desktop document pseudonymisation
application (private repository).

### Key features

- **Page orientation correction** — PP-LCNet_x1_0_doc_ori_onnx, 4 classes
  (0°/90°/180°/270°). Detects and corrects rotated scans automatically.
  **Enabled by default** through `OcrOptions::default()`. Requires
  `set_doc_orientation_model()` on `OcrLite`; with no model loaded the flag
  is silently ignored.
- **Text detection** — DBNet (PP-OCRv6 / PP-OCRv5), multi-oriented, any aspect ratio.
- **Text recognition** — PPLCNetV4 + LightSVTR + CTC/NRTR multi-head.
  - **PP-OCRv6** (default): **50 languages** in a single model — CH / EN / JP + 46 Latin
    (IT, FR, DE, ES, PT and 41 more). Three tiers: `tiny` (6 MB), `small`, `medium`.
  - **PP-OCRv5 Latin** (legacy): 6 EU languages, separate model.
- **Per-line orientation classifier** — 0°/180° per line (PP-OCRv2 cls).
- **Layout analysis** — PP-DocLayoutV3, 25 semantic classes (text, title, table,
  figure, header, footer, formula, list-item, …).
- **Table structure recognition** — SLANet_plus (488×488), HTML token stream
  → cell bounding boxes → GFM Markdown. Used through `TableStructureRecognizer`.
- **Table cell detection** — RT-DETR-L (wired + wireless), geometric grid → GFM Markdown.
- **Word-level boxes** — CTC timestep tracking → per-word bbox (highlighting / hit-testing).
- **Model auto-download** — `ModelHub` fetches the ONNX weights from HuggingFace
  on first use and keeps them in a local cache.
- **Cross-platform** — Windows (x86_64 + ARM64), macOS (Intel + Apple Silicon), Linux.

---

## The pure core (`src/core`)

Everything that decides **how a page is read** — layout decoding, NMS,
XY-Cut reading order, line-to-region association, the page hierarchy —
lives in `src/core` and depends on nothing but `serde`. It compiles to
`wasm32-unknown-unknown`.

That is deliberate. The same logic has to serve two very different
runtimes: a native one, where `ort` loads the ONNX models, and a browser
one, where the models run on `onnxruntime-web` and recognition happens in
JavaScript. Keeping two copies of those rules means watching them drift —
one threshold fixed here and not there, and the two products stop reading
the same page the same way. Here there is one copy.

```bash
# The rule is enforced by the compiler, not by convention:
cargo check --no-default-features --target wasm32-unknown-unknown
```

Anything requiring `ort`, `image` or an FFI sits behind the `native`
feature (on by default). Turn it off and the pure core is what remains.

### Guarantees of `associate_lines`

The function that assigns recognised lines to layout regions is the point
where a document either keeps all its text or quietly loses some:

- a line whose centroid falls in no region is attached to the **nearest**
  one; with no regions at all it still comes back, flagged as unassigned.
  **The layout never filters the text.** A wrong region costs a paragraph
  out of place; a dropped line costs text nobody will ever read again;
- ordering is the layout model's reading order first, then **XY-Cut**
  inside each region. XY-Cut picks the axis with the widest corridor
  rather than always cutting horizontally, which is what stops a
  two-column page from being read one line left, one line right.

Consumers get this by calling the function — there is no ordering policy
to re-implement per engine, and therefore no way for two engines to
disagree about the same page.

---

## Performance (release build, ARM64 Snapdragon X Elite)

Measured over 6 PDF pages (2 documents: an A4 paper and an IEEE template
with tables and rotations):

| Stage | Average time/page |
|---|---|
| PDF→PNG (pdftoppm 200 DPI) | ~1400 ms |
| Page orientation (PP-LCNet) | included in OCR |
| Layout (PP-DocLayoutV3) | **~820 ms** |
| OCR (PP-OCRv6 tiny det+rec) | **~1800 ms** |
| Table structure (SLANet_plus) | **~18 ms/table** |
| **Total pipeline/page** | **~2.6 s** |

> Debug build: layout ~3300 ms, OCR ~20000 ms — the release speedup is ~6-12×.

---

## Pipeline

```
Input image
    │
    ▼
┌──────────────────────────┐
│  PP-LCNet doc_ori        │  → detects rotation (0/90/180/270°)
│  (if use_doc_orientation)│    rotates the image before OCR
└──────────┬───────────────┘    OcrResult.page_angle = degrees corrected
           │
           ▼
┌─────────────────┐
│  DBNet (det)    │  → text polygon boxes
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Cls (orient.)  │  → flips lines 180° when rotated (per line)
└────────┬────────┘
         │
         ▼
┌─────────────────────────────────┐
│  PPLCNetV4 + SVTR-CTC (rec)     │  → strings + confidence (50 languages)
└────────┬────────────────────────┘
         │
         ├────►  Plain OCR result: Vec<TextBlock>
         │
         │  (optional layout-aware path:)
         ▼
┌─────────────────────┐
│ PP-DocLayoutV3      │  → 25-class regions w/ reading order
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  XY-Cut / sort      │  → TextBlocks tagged by SemanticClass
└────────┬────────────┘
         │  (for Table regions:)
         ├──────────────────────────────────────┐
         ▼                                      ▼
┌─────────────────────┐              ┌──────────────────────┐
│ SLANet_plus         │              │ RT-DETR-L cell det   │
│ (table structure)   │              │ (wired / wireless)   │
└────────┬────────────┘              └──────────┬───────────┘
         │ HTML token stream                    │ cell bbox
         ▼                                      ▼
┌─────────────────────┐              ┌──────────────────────┐
│ cell_boxes + OCR    │              │ derive_grid (geom.)  │
│ → GFM Markdown      │              │ → GFM Markdown       │
└─────────────────────┘              └──────────────────────┘
```

---

## Crate layout

```
src/
├── core/                — PURE core, no ort / no image / no FFI
│   ├── types.rs         — engine-agnostic Page → Block → Paragraph → Line → Word
│   ├── layout.rs        — LayoutModelSpec, decode, NMS, XY-Cut, associate_lines,
│   │                      build_page, column detection, heading / ocr-skip rules
│   ├── preprocess.rs    — bitmap clean-up, backend independent
│   ├── pdf.rs           — bitmap px → PDF page points, page_angle inversion
│   └── pdfwriter.rs     — searchable text layer
│
├── lib.rs               — public re-exports + module overview
├── model_hub.rs         — ONNX auto-download from HuggingFace (feature fetch-models)
├── ocr_lite.rs          — OcrLite orchestrator (det + cls + rec + doc_orientation)
├── db_net.rs            — DBNet text detection (vendored from paddle-ocr-rs)
├── crnn_net.rs          — PPLCNetV4/SVTR recognition (vendored)
├── angle_net.rs         — per-line orientation classifier (vendored)
├── ocr_utils.rs         — image preprocessing, perspective warp (vendored)
├── scale_param.rs       — resize-to-multiple-of-32 (vendored)
├── ocr_result.rs        — DTOs (TextBox, TextLine, OcrResult, WordBox)
├── ocr_error.rs         — unified error type via thiserror
├── compat.rs            — permanent shim for the API differences between ort release candidates
│
│   Added in this fork (not in paddle-ocr-rs upstream):
├── layout.rs            — PP-DocLayoutV3 (RT-DETR-style) layout analyzer + XY-Cut
├── cell_detection.rs    — RT-DETR-L cell detector + derive_grid → GFM helper
├── table_classifier.rs  — PP-LCNet: TableTypeClassifier (wired/wireless) +
│                          DocOrientationClassifier (0/90/180/270°)
├── table_structure.rs   — SLANet_plus / SLANeXt table structure recognizer
│                          → HTML token stream → TableCellBox → GFM Markdown
├── doc_unwarp.rs        — UVDoc document unwarping (reserved, enabled=false)
└── formula_rec.rs       — PP-FormulaNet-plus-L LaTeX recognition (reserved)

tests/
├── compat_pp_ocrv6.rs   — PP-OCRv6 × ort rc.13 compatibility smoke test
├── pipeline_layout.rs   — layout + OCR + XY-Cut over a multi-page TIFF (CER metric)
├── table_pipeline.rs    — table detection + SLANet_plus decoding over TIFF
└── art_pipeline.rs      — end-to-end pipeline over PDF (pdftoppm + orient + layout
                           + OCR + table structure); release vs debug benchmarking

examples/
├── rogito_v6.rs         — PP-OCRv6 OCR over a scanned notarial deed
└── cell_aware_reorder.rs — full pipeline + cell-aware reordering
```

---

## Models — PP-OCRv6 (default)

PP-OCRv6 is available on HuggingFace in three tiers. The ONNX models can be
fetched through `ModelHub::ensure()` or downloaded manually.

### Available tiers

| Tier    | det.onnx | rec.onnx | Total | Languages |
|---------|----------|----------|--------|--------|
| `Tiny`  | 1.8 MB   | 4.5 MB   | **~6 MB** | 50  |
| `Small` | ~6 MB    | ~20 MB   | ~26 MB | 50   |
| `Medium`| 62 MB    | 77 MB    | ~139 MB | 50  |

For deployment on ARM64 Snapdragon X Elite, **`Tiny`** is a reasonable
starting point (benchmark on your own machine before committing to a tier).

### PP-OCRv6 architecture

**Detection** (`PP-OCRv6_{tiny|small|medium}_det`):
- Backbone: `PPLCNetV4` (unified, new in v6)
- Neck: `RepLKPAN` (large-kernel convolutions)
- Head: `DBHead` + `DiceFocalLoss`
- Input: `[N, 3, H, W]` dynamic (min 32×32, opt 736×736)
- Normalisation: `mean=[0.485, 0.456, 0.406]`, `std=[0.229, 0.224, 0.225]`

**Recognition** (`PP-OCRv6_{tiny|small|medium}_rec`):
- Backbone: `PPLCNetV4 {tiny|small|medium}`
- Decoder: `LightSVTR` + `CTC + NRTR` multi-head
- Input: `[1, 3, 48, W]` dynamic (max W = 3200)
- Dictionary: `ppocrv6_dict.txt` — **18,708 unified characters** (50 languages)

Against PP-OCRv5: +5.1% recognition, +4.6% detection on an internal eval set.

---

## Auto-download with ModelHub

```rust
use ppocr_rs::{
    ModelHub, PpOcrVersion, PpStructureModel,
    OcrLite, OcrOptions, DocOrientationClassifier,
};

// 1. Fetch PP-OCRv6 tiny and the page orientation classifier.
//    Default cache: %LOCALAPPDATA%\ppocr-rs\models\ (Windows)
//                   ~/.cache/ppocr-rs/models/       (Linux/macOS)
let hub      = ModelHub::with_default_cache()?;
let paths    = hub.ensure(PpOcrVersion::V6Tiny)?;
let ori_paths = hub.ensure_single(PpStructureModel::DocOrientation)?;

// 2. Initialise OcrLite with the orientation classifier.
let mut ocr = OcrLite::new();
ocr.init_models_no_angle(
    paths.det_onnx.to_str().unwrap(),
    paths.rec_onnx.to_str().unwrap(),
    paths.dict_txt.to_str().unwrap(),
    4, // num_thread
)?;
// Load the page orientation classifier (PP-LCNet, 7 MB).
// OcrOptions::default() sets use_doc_orientation=true: if the model is
// not loaded, the flag is silently ignored.
let ori_clf = DocOrientationClassifier::from_path(&ori_paths.onnx)?;
ocr.set_doc_orientation_model(ori_clf);

// 3. OCR with automatic orientation correction.
let img    = image::open("scan.png")?.to_rgb8();
let result = ocr.detect_with_options(
    &img, 10, 960, 0.6, 0.3, 1.6, false, false,
    OcrOptions::default(), // use_doc_orientation=true by default
)?;
if result.page_angle != 0 {
    println!("Page rotated by {}° — corrected automatically", result.page_angle);
}
for line in &result.text_blocks {
    println!("{:.2}  {}", line.text_score, line.text);
}
```

> The download is **blocking**. In a GUI or async runtime, run it on a
> separate thread.

### Custom cache directory

```rust
let hub = ModelHub::new("/mnt/models/ppocr-rs");
let paths = hub.ensure(PpOcrVersion::V6Medium)?;
```

### Required feature flag

```toml
# Your project's Cargo.toml
[dependencies]
ppocr-rs = { version = "0.8.4", features = ["fetch-models"] }
```

Without `fetch-models`, `ensure()` returns `OcrError::ModelHubError` when the
files are not already cached. Useful for offline environments, or deployments
where the models are pre-installed.

---

## Manual download (alternative to ModelHub)

### PowerShell

```powershell
$HF = "https://huggingface.co/PaddlePaddle"
$DIR = "$env:LOCALAPPDATA\ppocr-rs\models\pp_ocrv6_tiny"
New-Item -ItemType Directory -Force $DIR | Out-Null

# Tiny (6 MB total):
curl -L -o "$DIR\det.onnx"  "$HF/PP-OCRv6_tiny_det_onnx/resolve/main/inference.onnx"
curl -L -o "$DIR\rec.onnx"  "$HF/PP-OCRv6_tiny_rec_onnx/resolve/main/inference.onnx"
curl -L -o "$DIR\dict.txt"  "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/v3.7.0/ppocr/utils/dict/ppocrv6_dict.txt"

# Or Medium (139 MB):
$DIR_M = "$env:LOCALAPPDATA\ppocr-rs\models\pp_ocrv6_medium"
New-Item -ItemType Directory -Force $DIR_M | Out-Null
curl -L -o "$DIR_M\det.onnx"  "$HF/PP-OCRv6_medium_det_onnx/resolve/main/inference.onnx"
curl -L -o "$DIR_M\rec.onnx"  "$HF/PP-OCRv6_medium_rec_onnx/resolve/main/inference.onnx"
curl -L -o "$DIR_M\dict.txt"  "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/v3.7.0/ppocr/utils/dict/ppocrv6_dict.txt"
```

### Legacy models (PP-OCRv5 Latin, 6 EU languages)

```powershell
$ONNX = "https://github.com/jingsongliujing/OnnxOCR/raw/main/onnxocr/models/ppocrv5"
$DIR = "models\latin"
New-Item -ItemType Directory -Force $DIR | Out-Null
curl -L -o "$DIR\det.onnx"       "$ONNX/det/PP-OCRv5_server_det.onnx"
curl -L -o "$DIR\rec_latin.onnx" "$ONNX/rec/latin_PP-OCRv5_mobile_rec.onnx"
curl -L -o "$DIR\dict_latin.txt" "$ONNX/rec/dict/latin_dict.txt"
```

### Layout + Table + Orientation (PP-DocLayoutV3 + SLANet_plus + PP-LCNet)

```powershell
$HF  = "https://huggingface.co/PaddlePaddle"
$B2O = "https://huggingface.co/datasets/kreuzberg-dev/paddle-to-onnx/resolve/main"
New-Item -ItemType Directory -Force models\layout, models\table, models\orientation | Out-Null

# Layout
curl -L -o models\layout\PP-DocLayoutV3.onnx "$B2O/PP-DocLayoutV3.onnx"

# Table cell detection (wired + wireless)
curl -L -o models\table\RT-DETR-L_wired_table_cell_det.onnx    "$B2O/RT-DETR-L_wired_table_cell_det.onnx"
curl -L -o models\table\RT-DETR-L_wireless_table_cell_det.onnx "$B2O/RT-DETR-L_wireless_table_cell_det.onnx"

# Table structure — SLANet_plus (488×488, via ModelHub or manually)
curl -L -o models\table\SLANet_plus.onnx "$HF/SLANet_plus_onnx/resolve/main/inference.onnx"
curl -L -o models\table\table_structure_dict.txt "$HF/SLANet_plus_onnx/resolve/main/table_structure_dict.txt"

# Page orientation (PP-LCNet, 224×224, 7 MB — via ModelHub or manually)
curl -L -o models\orientation\inference.onnx "$HF/PP-LCNet_x1_0_doc_ori_onnx/resolve/main/inference.onnx"
```

---

## API

### Plain OCR with automatic orientation (PP-OCRv6)

```rust
use ppocr_rs::{ModelHub, PpOcrVersion, PpStructureModel,
                     OcrLite, OcrOptions, DocOrientationClassifier};

let hub  = ModelHub::with_default_cache()?;
let ocr_paths = hub.ensure(PpOcrVersion::V6Tiny)?;
let ori_paths = hub.ensure_single(PpStructureModel::DocOrientation)?;

let mut ocr = OcrLite::new();
ocr.init_models_no_angle(
    ocr_paths.det_onnx.to_str().unwrap(),
    ocr_paths.rec_onnx.to_str().unwrap(),
    ocr_paths.dict_txt.to_str().unwrap(),
    4,
)?;
ocr.set_doc_orientation_model(
    DocOrientationClassifier::from_path(&ori_paths.onnx)?
);

let img    = image::open("scan.png")?.to_rgb8();
// OcrOptions::default() → use_doc_orientation=true
let result = ocr.detect_with_options(
    &img, 10, 960, 0.6, 0.3, 1.6, false, false, OcrOptions::default())?;

println!("Rotation corrected: {}°", result.page_angle);
for line in &result.text_blocks {
    println!("{:>5.2}  {:?}  {}", line.text_score, line.box_points, line.text);
}
```

### Layout-aware OCR

```rust
use ppocr_rs::{LayoutAnalyzer, OcrOptions};

let mut layout = LayoutAnalyzer::from_path("models/layout/PP-DocLayoutV3.onnx")?;
let aware = ocr.detect_with_layout(
    &img, &mut layout,
    10, 960, 0.6, 0.3, 1.6, false, false, OcrOptions::default(),
)?;

for blk in &aware.blocks {
    let cls = blk.layout_index
        .map(|i| format!("{:?}", aware.layout_boxes[i].semantic_class()))
        .unwrap_or_default();
    println!("[{cls}] {}", blk.block.text);
}
```

### Table structure recognition → GFM Markdown

```rust
use ppocr_rs::{TableStructureRecognizer, LayoutAnalyzer, LayoutClass};

let mut layout = LayoutAnalyzer::from_path("models/layout/PP-DocLayoutV3.onnx")?;
let recognizer = TableStructureRecognizer::from_path_with_dict(
    "models/table/SLANet_plus.onnx",
    Some(std::path::Path::new("models/table/table_structure_dict.txt")),
)?.with_input_size(488); // SLANet_plus uses 488×488; SLANeXt uses 512×512

let layout_boxes = layout.analyze(&img)?;
for lb in layout_boxes.iter().filter(|lb| lb.class == LayoutClass::Table) {
    let crop      = image::imageops::crop_imm(&img, lb.x, lb.y, lb.w, lb.h).to_image();
    let structure = recognizer.recognize(&crop)?;
    println!("Table: {} cells, score {:.3}", structure.cell_boxes.len(), structure.score);
    // structure.cell_boxes → TableCellBox { x1,y1,x2,y2 }
    // structure.html_tokens → HTML token string, for debugging
}
```

### Table cell detection + Markdown (geometric, via RT-DETR-L)

```rust
use ppocr_rs::{CellDetector, derive_grid, grid_to_gfm};

let detector = CellDetector::from_path(
    "models/table/RT-DETR-L_wired_table_cell_det.onnx")?;
let cells = detector.detect(&table_crop, 0.4)?;
let grid  = derive_grid(&cells);
let gfm   = grid_to_gfm(&grid, &lines_within_table);
println!("{gfm}");
```

### Word-level boxes

```rust
let opts = OcrOptions { return_word_box: true, ..OcrOptions::default() };
let result = ocr.detect_with_options(
    &img, 10, 960, 0.6, 0.3, 1.6, false, false, opts)?;
for blk in &result.text_blocks {
    for wb in &blk.words {
        println!("  '{}'  {:?}", wb.text, wb.box_points);
    }
}
```

### `OcrOptions` — available flags

| Flag | Default | Purpose |
|---|---|---|
| `use_doc_orientation` | **`true`** | Corrects page rotation (0/90/180/270°) when the model is loaded |
| `return_word_box` | `false` | Adds per-word bounding boxes via CTC timestep tracking |
| `lang` | `None` | Language routing (reserved — PP-OCRv6 already covers 50 languages) |
| `use_doc_unwarping` | `false` | UVDoc perspective unwarping (reserved) |
| `use_seal` | `false` | Circular stamps (reserved) |
| `use_formula` | `false` | PP-FormulaNet LaTeX (reserved) |

---

## Smoke test: PP-OCRv6 × ort rc.13

Before wiring PP-OCRv6 into the full pipeline, run the compatibility tests in
[`tests/compat_pp_ocrv6.rs`](tests/compat_pp_ocrv6.rs). They download the tiny
models (~6 MB) and check that ort rc.13 can load them and produce output of
the expected shape.

```powershell
# x86_64 (pyke prebuilt DLL):
cargo test --test compat_pp_ocrv6 --features test-binaries,fetch-models -- --nocapture

# ARM64 Snapdragon X Elite:
$env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
cargo test --test compat_pp_ocrv6 --features example-dynamic,fetch-models -- --nocapture
```

### End-to-end pipeline over PDF (`tests/art_pipeline.rs`)

A full test that processes real PDFs through `pdftoppm`, times each stage and
checks orientation + layout + OCR + table structure:

```powershell
# Release (recommended for benchmarking):
$env:ORT_DYLIB_PATH = "...\onnxruntime.dll"
cargo test --test art_pipeline --release --features example-dynamic,fetch-models -- --nocapture
```

---

## Build

### Default (CPU only)

```bash
cargo build --release
```

ONNX Runtime is **not** linked statically. On first run, `ort` looks for
`onnxruntime.dll` / `libonnxruntime.so` next to the binary. Two ways to
provide it:

1. **`test-binaries`** — downloads the pyke.io prebuilt DLL (x86_64 only):
   ```bash
   cargo test --features test-binaries
   ```

2. **`example-dynamic`** — dlopen at runtime (required on ARM64, where
   pyke.io publishes no binaries for the pinned release candidate):
   ```powershell
   $env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
   cargo run --example cell_aware_reorder --features example-dynamic
   ```

### Feature flags

| Flag | Effect |
|---|---|
| `fetch-models` | Enables `ModelHub` HTTP download through `ureq` |
| `test-binaries` | `ort/download-binaries` — x86_64 DLL for `cargo test` |
| `example-dynamic` | `ort/load-dynamic` — dlopen for ARM64 / offline deployment |
| `directml` | DirectML (Windows GPU) |
| `coreml` | CoreML (macOS Apple Silicon) |
| `cuda` | CUDA + cuDNN |
| `xnnpack` | XNNPACK CPU SIMD (ARM + x86) |

QNN (Snapdragon Hexagon NPU) is **excluded**: benchmarks on a Snapdragon X
Elite show that PP-OCRv5/v6 gains nothing from the NPU, because of dynamic
shapes and CPU-bound post-processing.

---

## What's new in 0.8

**Breaking change if you are coming from 0.7.x.**

- `ort` moves from `2.0.0-rc.9` to `2.0.0-rc.13`, and with it **the required
  runtime changes: ONNX Runtime 1.28**. Upgrading the crate without upgrading
  the shared library produces `BadVersion` when the first model is loaded.
- `ndarray` moves to 0.17, matching what ort pulls in. If two versions of
  `ndarray` end up in the dependency tree, tensors stop implementing
  `IntoValueTensor` and the error you get says nothing about versions — check
  with `cargo tree -i ndarray`.
- `try_extract_tensor` changed its return type, from `Result<ArrayViewD<T>>`
  to a `(&Shape, &[T])` tuple. The difference is confined to `src/compat.rs`,
  so it does not surface in the crate's public API.

The minor version bump is there precisely for this: the published 0.7.3 ran on
rc.9, and a patch release would have suggested that recompiling was enough.

---

## Differences from upstream `paddle-ocr-rs`

A fork of [meibel-ai/paddle-ocr-rs](https://github.com/meibel-ai/paddle-ocr-rs)
(branch `ort-rc11`, Apache-2.0), which derives from
[mg-chao/paddle-ocr-rs](https://github.com/mg-chao/paddle-ocr-rs).

| Change | Reason |
|---|---|
| `ort = =2.0.0-rc.13` (upstream: rc.11) | Exact pin: the API breaks between release candidates, so moving up has to be a decision rather than a side effect of `cargo update`. Requires ONNX Runtime 1.28. |
| `ndarray = 0.17` | Matches what ort rc.13 pulls in. Mixing versions produces `IntoValueTensor` errors that are hard to read. |
| `compat.rs` shim for `try_extract_tensor` | The return type changed again in rc.13: from `Result<ArrayViewD<T>>` to a `(&Shape, &[T])` tuple. The shim keeps the difference in one place. |
| Edition `2021` (was `2024`) | Compatibility with the workspace's minimum toolchain. |
| **`layout.rs`** added | PP-DocLayoutV3 (25 classes) + XY-Cut reading order. Not in upstream. |
| **`cell_detection.rs`** added | RT-DETR-L cell detector + grid derivation → GFM. Not in upstream. |
| **`table_classifier.rs`** added | PP-LCNet wired/wireless classifier + DocOrientationClassifier (224×224). |
| **`table_structure.rs`** added | SLANet_plus / SLANeXt structure recognizer → HTML tokens → cell bbox → GFM. |
| **`doc_unwarp.rs`** added | UVDoc document unwarping (reserved). |
| **`formula_rec.rs`** added | PP-FormulaNet-plus-L LaTeX output (reserved). |
| **`model_hub.rs`** added | ONNX auto-download from HuggingFace. Not in upstream. |
| **`OcrLite.set_doc_orientation_model()`** | Wires DocOrientationClassifier into the pipeline; switched on through `OcrOptions.use_doc_orientation` (default `true`). |
| **`OcrResult.page_angle`** added | Reports the rotation applied (0/90/180/270). |

---

## Compatibility matrix

| OS | Architecture | Status |
|---|---|---|
| Windows 10 / 11 | x86_64 | ✅ Production |
| Windows 11 | ARM64 (Snapdragon) | ✅ Production (`load-dynamic`) |
| macOS 13+ | Intel | ✅ Tested |
| macOS 13+ | Apple Silicon | ✅ Tested (CoreML EP) |
| Linux | x86_64 | ✅ Tested (Ubuntu 22.04+) |
| Linux | aarch64 | 🟡 Untested, expected to work |

| Runtime dependency | Version | Notes |
|---|---|---|
| ONNX Runtime | **1.28** | Mandatory. An older library fails with `BadVersion`. |
| `ort` | `=2.0.0-rc.13` | Exact pin: the API breaks between release candidates. |
| `ndarray` | 0.17 | Exactly one version in the tree (`cargo tree -i ndarray`). |

---

## Roadmap

- [ ] **ModelHub: SHA-256 checksums** — integrity check after download.
- [ ] **ModelHub: progress callback** — for GUIs / progress bars.
- [ ] **Per-word boxes on vertical text** — currently skipped when `crop_h >= crop_w`.
- [ ] **CJK / non-Latin models** — per-language routing when needed (PP-OCRv6
  already covers CH/JP in the same model; only the routing in `OcrOptions.lang`
  is missing).
- [ ] **`detect_with_layout` + orientation** — rotate before `layout.analyze()`
  so coordinates stay consistent when `use_doc_orientation=true`.
- [ ] **ByT5 OCR post-correction** — byte-level correction of common OCR errors
  (O↔0, dropped double consonants, missing spaces) between OCR and cache.

---

## Model sources

| Model | Source | Notes |
|---|---|---|
| PP-OCRv6 tiny/small/medium det+rec ONNX | `PaddlePaddle/PP-OCRv6_*_onnx` on HuggingFace | Apache-2.0. Fetched through `ModelHub`. |
| `ppocrv6_dict.txt` | [PaddleOCR v3.7.0](https://github.com/PaddlePaddle/PaddleOCR/tree/v3.7.0) | 18,708 characters. Apache-2.0. |
| PP-OCRv5 Latin (legacy) | [`jingsongliujing/OnnxOCR`](https://github.com/jingsongliujing/OnnxOCR) | 6 EU languages. Apache-2.0. |
| PP-DocLayoutV3 ONNX | [`kreuzberg-dev/paddle-to-onnx`](https://github.com/kreuzberg-dev/paddle-to-onnx) | 125 MB. Apache-2.0. |
| RT-DETR-L wired/wireless cell det | [`kreuzberg-dev/paddle-to-onnx`](https://github.com/kreuzberg-dev/paddle-to-onnx) | 123 MB each. Apache-2.0. |
| SLANet_plus ONNX + dict | `PaddlePaddle/SLANet_plus_onnx` on HuggingFace | 488×488 input. Fetched through `ModelHub`. |
| PP-LCNet_x1_0_doc_ori_onnx | `PaddlePaddle/PP-LCNet_x1_0_doc_ori_onnx` on HuggingFace | 7 MB, 224×224 input. Fetched through `ModelHub`. |
| PP-LCNet_x1_0_table_cls_onnx | `PaddlePaddle/PP-LCNet_x1_0_table_cls_onnx` on HuggingFace | 7 MB, 48×192 input. |
| Per-line orientation cls | [`jingsongliujing/OnnxOCR`](https://github.com/jingsongliujing/OnnxOCR) | PP-OCRv2 cls, < 1 MB. |

---

## Credits

- [`mg-chao/paddle-ocr-rs`](https://github.com/mg-chao/paddle-ocr-rs) — original Rust port (Apache-2.0)
- [`meibel-ai/paddle-ocr-rs`](https://github.com/meibel-ai/paddle-ocr-rs) — branch `ort-rc11` (Apache-2.0)
- [PaddlePaddle / PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) — PP-OCRv5/v6 models
- [PaddleX](https://github.com/PaddlePaddle/PaddleX) — PP-DocLayoutV3, SLANet_plus, PP-LCNet classifiers
- [`kreuzberg-dev/paddle-to-onnx`](https://github.com/kreuzberg-dev/paddle-to-onnx) — pre-converted ONNX
- [`jingsongliujing/OnnxOCR`](https://github.com/jingsongliujing/OnnxOCR) — alternative ONNX + dictionaries

## License

Apache License 2.0 — see [`LICENSE`](LICENSE) for the full text and
[`NOTICE`](NOTICE) for attribution. Vendored files keep their original
copyright headers.

The ONNX model weights are **not** distributed with this crate: they are
downloaded at runtime from their publishers and remain subject to their own
licences — see [Model sources](#model-sources).
