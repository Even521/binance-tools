//! Pipeline tabella completa:
//!   Layout (PP-DocLayoutV3) → crop Table box
//!   → SLANet_plus structure recognition
//!   → associazione testo OCR full-page → output Markdown.
//!
//! Esecuzione:
//! ```powershell
//! $env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
//! cargo test --test table_pipeline --features example-dynamic,fetch-models -- --nocapture
//! ```

use ppocr_rs::{
    model_hub::{ModelHub, PpOcrVersion},
    LayoutAnalyzer, LayoutBox, LayoutClass, OcrLite, OcrOptions, OcrResult,
    TableCellBox, TableStructure, TableStructureRecognizer,
};
use std::{path::Path, time::Instant};

// ─── Paths (configurare via env var) ─────────────────────────────────────────

fn tiff_dir() -> String {
    std::env::var("PPOCR_TEST_TIFF_DIR")
        .unwrap_or_else(|_| "tests/fixtures/tiff".to_string())
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

fn setup_table_recognizer() -> TableStructureRecognizer {
    let model = paddleocr_dir().join("table/SLANet_plus.onnx");
    let dict  = paddleocr_dir().join("table/table_structure_dict.txt");
    assert!(model.exists(), "SLANet_plus.onnx non trovato: {} (set PPOCR_MODELS_DIR)", model.display());
    assert!(dict.exists(),  "table_structure_dict.txt non trovato: {} (set PPOCR_MODELS_DIR)", dict.display());
    // SLANet_plus usa input 488×488 (SLANeXt usa 512×512)
    TableStructureRecognizer::from_path_with_dict(&model, Some(&dict))
        .expect("TableStructureRecognizer::from_path_with_dict")
        .with_input_size(488)
}

// ─── TIFF helpers ────────────────────────────────────────────────────────────

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
                let channels = data.len() / (w * h) as usize;
                let mut img = image::RgbImage::new(w, h);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let off = (y * w as usize + x) * channels;
                        let (r, g, b) = match channels {
                            1 => (data[off], data[off], data[off]),
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

// ─── Table pipeline helpers ───────────────────────────────────────────────────

/// Crop di un LayoutBox dall'immagine di pagina.
fn crop_layout_box(img: &image::RgbImage, lb: &LayoutBox) -> image::RgbImage {
    let x = lb.x.min(img.width().saturating_sub(1));
    let y = lb.y.min(img.height().saturating_sub(1));
    let w = lb.w.min(img.width().saturating_sub(x));
    let h = lb.h.min(img.height().saturating_sub(y));
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

/// Raccoglie il testo OCR che cade dentro una cell_box.
///
/// `cell` contiene coordinate relative al crop tabella → vanno traslate
/// in coordinate di pagina aggiungendo l'offset del layout box `(lb_x, lb_y)`.
fn collect_cell_text(
    ocr_result: &OcrResult,
    cell: &TableCellBox,
    lb_x: u32,
    lb_y: u32,
) -> String {
    let cx1 = lb_x as f32 + cell.x1;
    let cy1 = lb_y as f32 + cell.y1;
    let cx2 = lb_x as f32 + cell.x2;
    let cy2 = lb_y as f32 + cell.y2;

    let mut parts: Vec<(u32, String)> = Vec::new();
    for tb in &ocr_result.text_blocks {
        let pts = &tb.box_points;
        let bx = pts.iter().map(|p| p.x as f32).sum::<f32>() / pts.len() as f32;
        let by = pts.iter().map(|p| p.y as f32).sum::<f32>() / pts.len() as f32;
        if bx >= cx1 && bx <= cx2 && by >= cy1 && by <= cy2 {
            parts.push((by as u32, tb.text.trim().to_string()));
        }
    }
    parts.sort_by_key(|(y, _)| *y);
    parts
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Converte `TableStructure` + testo OCR full-page in una tabella GFM Markdown.
///
/// Non fa parsing dell'html token-by-token (la stringa html dipende dal
/// vocabolario del modello, che può essere sfasato). Usa invece direttamente
/// `cell_boxes` per le coordinate + conta le righe dall'html (`<tr>` count).
fn to_markdown_table(
    structure: &TableStructure,
    ocr_result: &OcrResult,
    lb: &LayoutBox,
) -> String {
    let cells = &structure.cell_boxes;
    if cells.is_empty() {
        return format!(
            "*(nessun cell_box) html_len={}*\nHTML: {}",
            structure.html_tokens.len(),
            &structure.html_tokens[..structure.html_tokens.len().min(200)]
        );
    }

    // Conta le righe dall'HTML (numero di <tr>)
    let n_rows = structure.html_tokens.matches("<tr>").count().max(1);
    let n_cols = (cells.len() + n_rows - 1) / n_rows;   // ceil div
    let n_cols = n_cols.max(1);

    // Raggruppa cell_boxes in righe per y-centroide:
    // ordina per y_centroide, poi spezza in chunks di n_cols.
    let mut ordered: Vec<(usize, f32, f32)> = cells.iter().enumerate()
        .map(|(i, c)| (i, (c.x1 + c.x2) * 0.5, (c.y1 + c.y2) * 0.5))
        .collect();
    ordered.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut rows: Vec<Vec<String>> = Vec::new();
    for chunk in ordered.chunks(n_cols) {
        let mut row_chunk = chunk.to_vec();
        row_chunk.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let row: Vec<String> = row_chunk.iter()
            .map(|&(i, _, _)| collect_cell_text(ocr_result, &cells[i], lb.x, lb.y))
            .collect();
        rows.push(row);
    }

    if rows.is_empty() {
        return "*(nessuna riga)*".to_string();
    }

    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut md = String::new();

    for (i, row) in rows.iter().enumerate() {
        md.push('|');
        for j in 0..max_cols {
            let c = row.get(j).map(String::as_str).unwrap_or("");
            md.push_str(&format!(" {} |", c.replace('|', "\\|")));
        }
        md.push('\n');
        if i == 0 {
            md.push('|');
            for _ in 0..max_cols { md.push_str("---|"); }
            md.push('\n');
        }
    }
    md
}

// ─── ONNX introspection ──────────────────────────────────────────────────────

/// Stampa nome e shape degli input/output del modello ONNX.
fn inspect_model_shapes(path: &str) {
    use ort::session::{builder::GraphOptimizationLevel, Session};
    let mut session = Session::builder()
        .expect("session builder")
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .expect("opt level")
        .commit_from_file(path)
        .expect("commit_from_file");

    eprintln!("  Inputs ({}):", session.inputs().len());
    for inp in session.inputs() {
        eprintln!("    {:?}  {:?}", inp.name(), inp.dtype());
    }
    eprintln!("  Outputs ({}):", session.outputs().len());
    for out in session.outputs() {
        eprintln!("    {:?}  {:?}", out.name(), out.dtype());
    }
}

// ─── Test ─────────────────────────────────────────────────────────────────────

#[test]
fn inspect_slanet_plus() {
    eprintln!("=== SLANet_plus.onnx ===");
    // Il path si risolve come negli altri test del file (rispetta
    // PPOCR_MODELS_DIR); il const `TABLE_MODEL` non è mai esistito.
    let model = paddleocr_dir().join("table/SLANet_plus.onnx");
    if !model.exists() {
        eprintln!("  modello non presente: {} — test saltato", model.display());
        return;
    }
    inspect_model_shapes(&model.to_string_lossy());
}

#[test]
fn pipeline_table_detection() {
    let dir_str  = tiff_dir();
    let tiff_dir_path = Path::new(&dir_str);
    if !tiff_dir_path.exists() {
        eprintln!("[skip] PPOCR_TEST_TIFF_DIR non trovata: {} — imposta la var d'ambiente", dir_str);
        return;
    }

    let mut tiff_files: Vec<_> = std::fs::read_dir(tiff_dir_path)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("tiff") | Some("tif") | Some("TIFF")
        ))
        .collect();
    tiff_files.sort();
    assert!(!tiff_files.is_empty(), "nessun TIFF in {dir_str}");

    let mut ocr      = setup_ocr();
    let mut layout   = setup_layout();
    let mut recognizer = setup_table_recognizer();
    eprintln!("[TableRec] vocab_size={}", recognizer.vocab_size());

    let mut total_tables_found   = 0usize;
    let mut total_tables_decoded = 0usize;

    for tiff_path in &tiff_files {
        let fname = tiff_path.file_name().unwrap().to_str().unwrap();
        eprintln!("\n══════ {fname} ══════");

        let pages = decode_tiff_pages(tiff_path);
        eprintln!("  {} pagine  {}×{}", pages.len(), pages[0].width(), pages[0].height());

        for (page_idx, page_img) in pages.iter().enumerate() {
            let page_num = page_idx + 1;

            // ── Layout detection ────────────────────────────────────────────
            let t0 = Instant::now();
            let layout_boxes = layout.analyze(page_img).expect("layout.analyze");
            let ms_layout = t0.elapsed().as_millis();

            // ── Filtra box Table ────────────────────────────────────────────
            let table_boxes: Vec<&LayoutBox> = layout_boxes.iter()
                .filter(|lb| lb.class == LayoutClass::Table)
                .collect();

            eprintln!(
                "  Pag {page_num}: {} layout box, {} Table  (layout={ms_layout}ms)",
                layout_boxes.len(), table_boxes.len()
            );

            if table_boxes.is_empty() { continue; }

            // ── OCR full-page ───────────────────────────────────────────────
            let t1 = Instant::now();
            let ocr_result = ocr
                .detect_with_options(
                    page_img, 50, 960, 0.5, 0.3, 1.6,
                    false, false, OcrOptions::default(),
                )
                .expect("detect_with_options");
            let ms_ocr = t1.elapsed().as_millis();
            eprintln!("  OCR: {} text blocks  (ocr={ms_ocr}ms)", ocr_result.text_blocks.len());

            // ── Per ogni tabella rilevata ─────────────────────────────────
            for (ti, lb) in table_boxes.iter().enumerate() {
                total_tables_found += 1;
                eprintln!(
                    "\n  ─── Tabella #{} (score={:.2}  x={} y={} w={} h={}) ───",
                    ti + 1, lb.score, lb.x, lb.y, lb.w, lb.h
                );

                let crop = crop_layout_box(page_img, lb);
                eprintln!("  crop: {}×{}", crop.width(), crop.height());

                // ── SLANet+ structure recognition ───────────────────────
                let t2 = Instant::now();
                match recognizer.recognize(&crop) {
                    Ok(structure) => {
                        let ms_rec = t2.elapsed().as_millis();
                        eprintln!(
                            "  SLANet+: score={:.3}  cells={}  html_len={}  (rec={ms_rec}ms)",
                            structure.score,
                            structure.cell_boxes.len(),
                            structure.html_tokens.len()
                        );
                        // Stampa HTML tokens (troncati a 300 char)
                        let html_preview = &structure.html_tokens
                            [..structure.html_tokens.len().min(300)];
                        eprintln!("  HTML: {html_preview}");

                        // ── Ricostruisci Markdown ───────────────────────
                        let md = to_markdown_table(&structure, &ocr_result, lb);
                        let md_lines: Vec<&str> = md.lines().collect();
                        let preview = md_lines.iter().take(25).cloned().collect::<Vec<_>>().join("\n");
                        eprintln!("  Markdown:\n{preview}");
                        if md_lines.len() > 25 {
                            eprintln!("  ... ({} righe totali)", md_lines.len());
                        }

                        total_tables_decoded += 1;
                    }
                    Err(e) => {
                        eprintln!("  WARN SLANet+ errore: {e}");
                    }
                }
            }
        }
    }

    eprintln!(
        "\n══ TOTALE: {total_tables_found} tabelle trovate, {total_tables_decoded} decodificate ══"
    );

    assert!(
        total_tables_found >= 1,
        "nessuna tabella rilevata nei TIFF in {dir_str}"
    );
    assert!(
        total_tables_decoded >= 1,
        "nessuna tabella decodificata da SLANet+ (trovate {total_tables_found})"
    );
}
