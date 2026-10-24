//! ppocr-rs — pipeline OCR PaddleOCR (PP-OCRv6) in puro Rust su
//! ort 2.0.0-rc.9, downportato da meibel-ai/paddle-ocr-rs (Apache-2.0).
//!
//! ## Componenti
//!
//! - [`db_net`]      — DBNet text detection (input immagine, output bbox).
//! - [`crnn_net`]    — CRNN/SVTR text recognition (input crop, output stringa).
//! - [`angle_net`]   — Classificatore orientamento per-line (0°/180°).
//! - [`layout`]      — PP-DocLayoutV3 layout analysis (NOSTRA aggiunta — non
//!                     presente in paddle-ocr-rs upstream).
//! - [`ocr_lite`]    — Pipeline orchestrator det → cls → rec.
//! - [`ocr_utils`]   — Helper image preprocessing (mean/std normalize, perspective warp).
//! - [`scale_param`] — Calcolo dimensioni resize-to-multiple-of-32.
//! - [`ocr_result`]  — DTO: TextBox, TextLine, OcrResult.
//! - [`ocr_error`]   — Error type unico via `thiserror`.
//! - [`compat`]      — Shim per le differenze API ort rc.9 vs rc.11.
//!
//! ## Differenze rispetto a paddle-ocr-rs upstream
//!
//! 1. **`ort` pinnato a `=2.0.0-rc.9`** — versione target permanente del
//!    workspace (rc.11+ si blocca su ARM64 Snapdragon X Elite).
//! 2. **`ndarray` a `0.16`** (re-export di rc.9) invece di `0.17` (rc.11).
//! 3. **API `try_extract_tensor`** convertita via [`compat`] (shim permanente).
//! 4. **Modulo [`layout`]** aggiunto per supportare PP-DocLayoutV3 — non
//!    presente in upstream.
//! 5. **Edition `2021`** invece di `2024` (per compat con il toolchain
//!    minimo del workspace).

#![allow(clippy::too_many_arguments)]

// ── Nucleo puro ────────────────────────────────────────────────────────
// Compila sempre, anche su wasm32: dipende solo da serde. Contiene la
// logica che nativo e browser devono condividere per forza — decodifica
// del layout, regole di lettura, gerarchia della pagina — perche' due
// copie divergono e i due prodotti smettono di leggere allo stesso modo.
pub mod core;

// ── Backend nativo (feature `native`, attiva di default) ───────────────
// Tutto cio' che richiede ort, image o una FFI. Disattivandolo resta il
// solo nucleo puro, che e' quello che attraversa wasm32.
#[cfg(feature = "native")] pub mod angle_net;
#[cfg(feature = "native")] pub mod base_net;
#[cfg(feature = "native")] pub mod compat;
#[cfg(feature = "native")] pub mod crnn_net;
#[cfg(feature = "native")] pub mod db_net;
#[cfg(feature = "native")] pub mod layout;
#[cfg(feature = "native")] pub mod model_hub;
#[cfg(feature = "native")] pub mod ocr_error;
#[cfg(feature = "native")] pub mod ocr_lite;
#[cfg(feature = "native")] pub mod ocr_result;
#[cfg(feature = "native")] pub mod ocr_utils;
#[cfg(feature = "native")] pub mod scale_param;
#[cfg(feature = "native")] pub mod cell_detection;
#[cfg(feature = "native")] pub mod doc_unwarp;
#[cfg(feature = "native")] pub mod formula_rec;
#[cfg(feature = "native")] pub mod table_classifier;
#[cfg(feature = "native")] pub mod table_structure;

#[cfg(feature = "native")] pub use ocr_error::OcrError;
#[cfg(feature = "native")] pub use ocr_lite::{LayoutAwareResult, OcrLite, OcrOptions, TextBlockWithLayout};
#[cfg(feature = "native")] pub use ocr_result::{Angle, OcrResult, Point, TextBlock, TextBox, TextLine, WordBox};
#[cfg(feature = "native")] pub use layout::{LayoutAnalyzer, LayoutBox, LayoutClass, SemanticClass, LAYOUT_INPUT_SIZE, xy_cut_order};
#[cfg(feature = "native")] pub use cell_detection::{CellDetector, CellBbox, derive_grid, grid_to_gfm, CELL_DETECT_INPUT_SIZE};
#[cfg(feature = "native")] pub use model_hub::{ModelHub, ModelPaths, PpOcrVersion, PpStructureModel, StructureModelPaths};
#[cfg(feature = "native")] pub use table_classifier::{TableTypeClassifier, TableType, DocOrientationClassifier, DocOrientation};
#[cfg(feature = "native")] pub use table_structure::{TableStructureRecognizer, TableStructure, TableCellBox, SLANEXT_INPUT_SIZE};
#[cfg(feature = "native")] pub use doc_unwarp::{DocUnwarper, UnwarpResult};
#[cfg(feature = "native")] pub use formula_rec::{FormulaRecognizer, FormulaResult, FORMULA_INPUT_SIZE};

// Il nucleo puro si raggiunge anche dalla radice: sono i tipi che
// attraversano il confine fra nativo e wasm.
pub use core::{BoundingBox, Block, Hierarchy, Line, Page, Paragraph, Word};
pub use core::layout::{LayoutModelSpec, LayoutBox as CoreLayoutBox};
