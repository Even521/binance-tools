//! Smoke test OCR su immagini reali (JPEG/PNG).
//!
//! Esecuzione:
//! ```powershell
//! # ARM64 Snapdragon X Elite:
//! $env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
//! cargo test --test lettera_ocr --features example-dynamic,fetch-models -- --nocapture
//!
//! # x86_64:
//! cargo test --test lettera_ocr --features test-binaries,fetch-models -- --nocapture
//! ```

use ppocr_rs::{
    doc_unwarp::DocUnwarper,
    model_hub::{ModelHub, PpOcrVersion, PpStructureModel},
    OcrLite, OcrOptions,
};

fn lettera_dir() -> String {
    std::env::var("PPOCR_TEST_LETTERA_DIR")
        .unwrap_or_else(|_| "tests/fixtures/images".to_string())
}

fn collect_images() -> Vec<std::path::PathBuf> {
    let dir_str = lettera_dir();
    let dir = std::path::Path::new(&dir_str);
    if !dir.exists() {
        eprintln!("[skip] PPOCR_TEST_LETTERA_DIR non trovata: {} — imposta la var d'ambiente", dir_str);
        return Vec::new();
    }

    let mut images: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("read_dir images")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("jpg") | Some("jpeg") | Some("png") | Some("JPEG") | Some("JPG")
            )
        })
        .collect();
    images.sort();
    images
}

fn setup_ocr() -> OcrLite {
    let hub = ModelHub::with_default_cache().expect("cache dir");
    let paths = hub.ensure(PpOcrVersion::V6Tiny).expect("download modelli OCR");

    let mut ocr = OcrLite::new();
    // cls/angle model non presente in ModelPaths — usiamo init_no_angle +
    // do_angle=false (angle_net.session resta None, non viene chiamata).
    ocr.init_models_no_angle(
        paths.det_onnx.to_str().unwrap(),
        paths.rec_onnx.to_str().unwrap(),
        paths.dict_txt.to_str().unwrap(),
        4,
    )
    .expect("init_models_no_angle");
    ocr
}

fn run_ocr(ocr: &mut OcrLite, img: &image::RgbImage) -> String {
    let result = ocr
        .detect_with_options(
            img,
            50,    // padding
            960,   // max_side_len
            0.5,   // box_score_thresh
            0.3,   // box_thresh
            1.6,   // un_clip_ratio
            false, // do_angle — angle_net non inizializzata
            false, // most_angle
            OcrOptions::default(),
        )
        .expect("detect_with_options");

    result
        .text_blocks
        .iter()
        .map(|b| b.text.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Test 1: OCR baseline ─────────────────────────────────────────────────────

#[test]
fn ocr_lettera_immagini() {
    let images = collect_images();
    if images.is_empty() { return; }
    eprintln!("=== {} immagini (baseline senza UVDoc) ===", images.len());

    let mut ocr = setup_ocr();

    for img_path in &images {
        eprintln!("\n─── {:?} ───", img_path.file_name().unwrap());
        let img = image::open(img_path).expect("open").to_rgb8();
        let text = run_ocr(&mut ocr, &img);
        eprintln!("{text}");
        eprintln!("─── fine ({} caratteri) ───", text.len());

        assert!(
            text.len() >= 10,
            "testo troppo corto ({} car) per {:?}",
            text.len(),
            img_path.file_name().unwrap()
        );
    }
}

// ─── Test 2: UVDoc → OCR ──────────────────────────────────────────────────────

#[test]
fn ocr_lettera_con_uvdoc() {
    let images = collect_images();
    if images.is_empty() { return; }
    eprintln!("=== {} immagini (UVDoc abilitato) ===", images.len());

    // Scarica UVDoc (~150 MB al primo run, poi riusa cache).
    let hub = ModelHub::with_default_cache().expect("cache dir");
    let uvdoc_paths = hub
        .ensure_single(PpStructureModel::DocUnwarp)
        .expect("download UVDoc");

    let mut unwarper = DocUnwarper::from_path(&uvdoc_paths.onnx).expect("DocUnwarper::from_path");
    unwarper.enabled = true;
    eprintln!("[UVDoc] modello caricato (shape dinamica — input a piena risoluzione)");

    let mut ocr = setup_ocr();

    // Directory per le immagini processate.
    let dir_str = lettera_dir();
    let out_dir = std::path::Path::new(&dir_str).join("uvdoc_out");
    std::fs::create_dir_all(&out_dir).expect("crea uvdoc_out");

    for img_path in &images {
        let fname = img_path.file_name().unwrap().to_str().unwrap();
        eprintln!("\n─── {fname} ───");

        let img = image::open(img_path).expect("open").to_rgb8();
        eprintln!("  originale: {}×{}", img.width(), img.height());

        // ── UVDoc ──────────────────────────────────────────────────────────
        let unwarp = unwarper.unwarp(&img).expect("DocUnwarper::unwarp");
        eprintln!(
            "  raddrizzata: {}×{}  (direct_output={})",
            unwarp.image.width(),
            unwarp.image.height(),
            unwarp.is_direct_output
        );

        // Salva l'immagine raddrizzata per ispezione visiva.
        let stem = std::path::Path::new(fname)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        let out_path = out_dir.join(format!("{stem}_uvdoc.jpg"));
        unwarp
            .image
            .save(&out_path)
            .unwrap_or_else(|e| eprintln!("  WARN: salvataggio fallito: {e}"));
        eprintln!("  salvata → {}", out_path.display());

        // ── OCR sull'immagine raddrizzata ──────────────────────────────────
        let text = run_ocr(&mut ocr, &unwarp.image);
        eprintln!("{text}");
        eprintln!("─── fine ({} caratteri) ───", text.len());

        assert!(
            text.len() >= 10,
            "testo troppo corto dopo UVDoc ({} car) per {fname}",
            text.len()
        );
    }

    eprintln!("\nImmagini raddrizzate salvate in: {}", out_dir.display());
}
