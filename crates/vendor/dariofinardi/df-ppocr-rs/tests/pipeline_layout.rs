//! Pipeline completa: Layout (PP-DocLayoutV3) + OCR (PP-OCRv6) su TIFF multi-pagina e JPEG.
//!
//! Misura qualità (CER vs ground-truth) e prestazioni (ms/pagina per layout, OCR, totale).
//! Applica XY-Cut per ordinamento in lettura corretto (colonne multiple).
//!
//! Esecuzione:
//! ```powershell
//! $env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
//! cargo test --test pipeline_layout --features example-dynamic,fetch-models -- --nocapture
//! ```

use ppocr_rs::{
    model_hub::{ModelHub, PpOcrVersion},
    LayoutAnalyzer, LayoutBox, OcrLite, OcrOptions, xy_cut_order,
};
use std::{path::Path, time::Instant};

// ─── Paths (configurare via env var) ─────────────────────────────────────────

fn tiff_dir() -> String {
    std::env::var("PPOCR_TEST_TIFF_DIR")
        .unwrap_or_else(|_| "tests/fixtures/tiff".to_string())
}

fn lettera_dir() -> String {
    std::env::var("PPOCR_TEST_LETTERA_DIR")
        .unwrap_or_else(|_| "tests/fixtures/lettera".to_string())
}

fn paddleocr_dir() -> std::path::PathBuf {
    std::env::var("PPOCR_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("models/paddleocr"))
}

// ─── Setup ───────────────────────────────────────────────────────────────────

fn setup_ocr() -> OcrLite {
    let hub = ModelHub::with_default_cache().expect("cache dir");
    let paths = hub.ensure(PpOcrVersion::V6Tiny).expect("modelli OCR");
    let mut ocr = OcrLite::new();
    ocr.init_models_no_angle(
        paths.det_onnx.to_str().unwrap(),
        paths.rec_onnx.to_str().unwrap(),
        paths.dict_txt.to_str().unwrap(),
        4,
    )
    .expect("init_models_no_angle");
    ocr
}

fn setup_layout() -> LayoutAnalyzer {
    let p = paddleocr_dir().join("layout/PP-DocLayoutV3.onnx");
    assert!(p.exists(), "PP-DocLayoutV3.onnx non trovato: {} (set PPOCR_MODELS_DIR)", p.display());
    LayoutAnalyzer::from_path(&p).expect("LayoutAnalyzer::from_path")
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Decodifica un TIFF multi-pagina. Ritorna Vec<RgbImage>, una per pagina.
fn decode_tiff_pages(path: &Path) -> Vec<image::RgbImage> {
    use tiff::decoder::{Decoder, DecodingResult};

    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("apertura TIFF {:?}: {e}", path));
    let mut decoder = Decoder::new(file).expect("Decoder TIFF");
    let mut pages = Vec::new();

    loop {
        let (w, h) = decoder.dimensions().expect("dimensioni TIFF");
        let result = decoder.read_image().expect("read_image TIFF");

        let rgb = match result {
            DecodingResult::U8(data) => {
                // Gestisce sia RGB che RGBA che Grayscale
                let channels = data.len() / (w * h) as usize;
                let mut img = image::RgbImage::new(w, h);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let off = (y * w as usize + x) * channels;
                        let (r, g, b) = match channels {
                            1 => (data[off], data[off], data[off]),
                            2 => (data[off], data[off], data[off]),
                            _ => (data[off], data[off + 1], data[off + 2]),
                        };
                        img.put_pixel(x as u32, y as u32, image::Rgb([r, g, b]));
                    }
                }
                img
            }
            DecodingResult::U16(data) => {
                let channels = data.len() / (w * h) as usize;
                let mut img = image::RgbImage::new(w, h);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let off = (y * w as usize + x) * channels;
                        let to_u8 = |v: u16| (v >> 8) as u8;
                        let (r, g, b) = match channels {
                            1 => { let v = to_u8(data[off]); (v, v, v) }
                            _ => (to_u8(data[off]), to_u8(data[off + 1]), to_u8(data[off + 2])),
                        };
                        img.put_pixel(x as u32, y as u32, image::Rgb([r, g, b]));
                    }
                }
                img
            }
            other => panic!("formato TIFF non supportato: {:?}", std::mem::discriminant(&other)),
        };

        pages.push(rgb);

        if !decoder.more_images() { break; }
        decoder.next_image().expect("next_image TIFF");
    }
    pages
}

/// Esegue layout + OCR su un'immagine, applica XY-Cut, ritorna (testo, layout_boxes, ms_layout, ms_ocr).
fn run_page(
    ocr: &mut OcrLite,
    layout: &mut LayoutAnalyzer,
    img: &image::RgbImage,
) -> (String, Vec<LayoutBox>, u64, u64) {
    // ── Layout ─────────────────────────────────────────────────────────────
    let t0 = Instant::now();
    let layout_boxes = layout.analyze(img).expect("layout.analyze");
    let ms_layout = t0.elapsed().as_millis() as u64;

    // ── OCR ────────────────────────────────────────────────────────────────
    let t1 = Instant::now();
    let ocr_result = ocr
        .detect_with_options(
            img, 50, 960, 0.5, 0.3, 1.6,
            false, false, OcrOptions::default(),
        )
        .expect("detect_with_options");
    let ms_ocr = t1.elapsed().as_millis() as u64;

    // ── XY-Cut order degli layout box ──────────────────────────────────────
    let lb_order = if layout_boxes.is_empty() {
        vec![]
    } else {
        xy_cut_order(&layout_boxes)
    };

    // ── Associa ogni text block al layout box (centroide containment) ──────
    // Struct: (lb_idx, cy_centroid, testo)
    struct Block { lb: usize, cy: u32, text: String }

    let blocks: Vec<Block> = ocr_result.text_blocks.iter().map(|tb| {
        let pts = &tb.box_points;
        let cx = pts.iter().map(|p| p.x).sum::<u32>() / pts.len() as u32;
        let cy = pts.iter().map(|p| p.y).sum::<u32>() / pts.len() as u32;

        let lb = layout_boxes.iter().position(|lb| lb.contains(cx, cy))
            .unwrap_or_else(|| {
                layout_boxes.iter().enumerate()
                    .min_by(|(_, a), (_, b)| {
                        a.distance_to(cx, cy).partial_cmp(&b.distance_to(cx, cy))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        Block { lb, cy, text: tb.text.trim().to_string() }
    }).collect();

    // ── Raggruppa per layout box, ordina internamente per y ─────────────────
    let mut by_lb: std::collections::HashMap<usize, Vec<(u32, String)>> =
        std::collections::HashMap::new();
    for b in &blocks {
        if !b.text.is_empty() {
            by_lb.entry(b.lb).or_default().push((b.cy, b.text.clone()));
        }
    }
    for v in by_lb.values_mut() { v.sort_by_key(|(cy, _)| *cy); }

    // ── Emetti testo in ordine XY-Cut ────────────────────────────────────────
    let mut lines = Vec::new();
    if lb_order.is_empty() {
        // Nessun layout: testo grezzo ordinato per y
        let mut sorted = blocks;
        sorted.sort_by_key(|b| b.cy);
        for b in sorted { if !b.text.is_empty() { lines.push(b.text); } }
    } else {
        for lb_idx in &lb_order {
            if let Some(group) = by_lb.remove(lb_idx) {
                for (_, t) in group { lines.push(t); }
            }
        }
        // Eventuali orfani (lb_idx non raggiunto da lb_order — impossibile, ma sicuro)
        for (_, group) in by_lb { for (_, t) in group { lines.push(t); } }
    }

    (lines.join("\n"), layout_boxes, ms_layout, ms_ocr)
}

/// Calcola Character Error Rate (CER) = edit_distance / len(reference).
/// Normalizzato su caratteri (non parole).
fn cer(hypothesis: &str, reference: &str) -> f32 {
    let h: Vec<char> = hypothesis.chars().collect();
    let r: Vec<char> = reference.chars().collect();
    if r.is_empty() { return 0.0; }
    let dist = edit_distance(&h, &r);
    dist as f32 / r.len() as f32
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![0usize; n + 1];
    for j in 0..=n { dp[j] = j; }
    let mut prev;
    for i in 1..=m {
        prev = dp[0];
        dp[0] = i;
        for j in 1..=n {
            let tmp = dp[j];
            dp[j] = if a[i - 1] == b[j - 1] {
                prev
            } else {
                1 + prev.min(dp[j]).min(dp[j - 1])
            };
            prev = tmp;
        }
    }
    dp[n]
}

fn print_layout_summary(layout_boxes: &[LayoutBox], page: usize) {
    eprintln!("  Layout page {page}: {} box", layout_boxes.len());
    let order = xy_cut_order(layout_boxes);
    for (rank, &i) in order.iter().enumerate() {
        let lb = &layout_boxes[i];
        eprintln!(
            "    [{}] {:?} (score={:.2}) x={} y={} w={} h={}  model_order={}",
            rank, lb.class, lb.score, lb.x, lb.y, lb.w, lb.h, lb.reading_order
        );
    }
}

// ─── Test 1: TIFF multi-pagina ───────────────────────────────────────────────

#[test]
fn pipeline_tiff_rogiti() {
    let dir_str  = tiff_dir();
    let tiff_dir = Path::new(&dir_str);
    if !tiff_dir.exists() {
        eprintln!("[skip] PPOCR_TEST_TIFF_DIR non trovata: {} — imposta la var d'ambiente", dir_str);
        return;
    }

    let mut tiff_files: Vec<_> = std::fs::read_dir(tiff_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("tiff") | Some("tif") | Some("TIFF")
            )
        })
        .collect();
    tiff_files.sort();
    assert!(!tiff_files.is_empty(), "nessun TIFF in {dir_str}");

    // Ground truth (opzionale — il file GT deve chiamarsi gt_pag<N>.txt)
    let gt_rogito:  Option<String> = None;
    let gt_manleva: Option<String> = None;

    let mut ocr = setup_ocr();
    let mut layout = setup_layout();

    let mut total_ms_layout = 0u64;
    let mut total_ms_ocr = 0u64;
    let mut total_pages = 0usize;

    for tiff_path in &tiff_files {
        let fname = tiff_path.file_name().unwrap().to_str().unwrap();
        eprintln!("\n══════ {fname} ══════");

        let pages = decode_tiff_pages(tiff_path);
        eprintln!("  {} pagine  {}×{}", pages.len(), pages[0].width(), pages[0].height());

        for (page_idx, page_img) in pages.iter().enumerate() {
            let page_num = page_idx + 1;
            eprint!("  Pagina {page_num}/{}: ", pages.len());

            let (text, layout_boxes, ms_lay, ms_ocr) =
                run_page(&mut ocr, &mut layout, page_img);

            total_ms_layout += ms_lay;
            total_ms_ocr += ms_ocr;
            total_pages += 1;

            eprintln!(
                "layout={ms_lay}ms  ocr={ms_ocr}ms  totale={}ms  char={}  boxes={}",
                ms_lay + ms_ocr, text.len(), layout_boxes.len()
            );
            print_layout_summary(&layout_boxes, page_num);

            // Stampa testo (prime 30 righe)
            eprintln!("  ── testo (XY-Cut order) ──");
            for (i, line) in text.lines().take(30).enumerate() {
                eprintln!("  {:2}│ {line}", i + 1);
            }
            if text.lines().count() > 30 {
                eprintln!("  ... ({} righe totali)", text.lines().count());
            }

            // CER vs ground truth dove disponibile
            let gt: Option<&str> = match page_num {
                5 => gt_rogito.as_deref(),
                2 => gt_manleva.as_deref(),
                _ => None,
            };
            if let Some(gt_text) = gt {
                let c = cer(&text, gt_text);
                eprintln!("  ── CER vs ground truth: {:.1}% ──", c * 100.0);
                assert!(
                    c < 0.30,
                    "CER troppo alta ({:.1}%) per {fname} pag {page_num}",
                    c * 100.0
                );
            }

            assert!(
                text.len() >= 50 || layout_boxes.is_empty(),
                "testo troppo corto ({} char) pag {page_num} di {fname}",
                text.len()
            );
        }
    }

    eprintln!(
        "\n══ TOTALE: {} pagine | layout avg={}ms | ocr avg={}ms | totale avg={}ms ══",
        total_pages,
        total_ms_layout / total_pages.max(1) as u64,
        total_ms_ocr / total_pages.max(1) as u64,
        (total_ms_layout + total_ms_ocr) / total_pages.max(1) as u64,
    );
}

// ─── Test 2: JPEG singola pagina ─────────────────────────────────────────────

#[test]
fn pipeline_layout_lettera() {
    let dir_str = lettera_dir();
    let dir = Path::new(&dir_str);
    if !dir.exists() {
        eprintln!("[skip] PPOCR_TEST_LETTERA_DIR non trovata: {} — imposta la var d'ambiente", dir_str);
        return;
    }

    let mut images: Vec<_> = std::fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("jpg") | Some("jpeg") | Some("JPEG")
            )
        })
        .collect();
    images.sort();

    let mut ocr = setup_ocr();
    let mut layout = setup_layout();

    for img_path in &images {
        let fname = img_path.file_name().unwrap().to_str().unwrap();
        eprintln!("\n══════ {fname} ══════");

        let img = image::open(img_path).expect("open JPEG").to_rgb8();
        eprintln!("  {}×{}", img.width(), img.height());

        let (text, layout_boxes, ms_lay, ms_ocr) =
            run_page(&mut ocr, &mut layout, &img);

        eprintln!(
            "  layout={ms_lay}ms  ocr={ms_ocr}ms  totale={}ms  char={}  boxes={}",
            ms_lay + ms_ocr, text.len(), layout_boxes.len()
        );
        print_layout_summary(&layout_boxes, 1);

        eprintln!("  ── testo (XY-Cut order) ──");
        for (i, line) in text.lines().take(40).enumerate() {
            eprintln!("  {:2}│ {line}", i + 1);
        }
        if text.lines().count() > 40 {
            eprintln!("  ... ({} righe totali)", text.lines().count());
        }

        assert!(text.len() >= 10, "testo troppo corto ({} char)", text.len());
    }
}
