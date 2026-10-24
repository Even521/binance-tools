//! Pipeline end-to-end su cartelle PDF:
//!   PDF → pdftoppm PNG → Layout (PP-DocLayoutV3) + OCR (PP-OCRv6)
//!   + TableStructure (SLANet_plus) → testo XY-Cut + tabelle Markdown.
//!
//! Misura il tempo di esecuzione per fase e per pagina.
//!
//! Esecuzione:
//! ```powershell
//! $env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
//! cargo test --test art_pipeline --features example-dynamic,fetch-models -- --nocapture
//! ```

use ppocr_rs::{
    model_hub::{ModelHub, PpOcrVersion, PpStructureModel},
    DocOrientationClassifier,
    LayoutAnalyzer, LayoutBox, LayoutClass, OcrLite, OcrOptions, OcrResult,
    TableCellBox, TableStructure, TableStructureRecognizer, xy_cut_order,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

// ─── Paths (configurare via env var) ─────────────────────────────────────────

fn tests_root() -> String {
    std::env::var("PPOCR_TEST_PDF_DIR")
        .unwrap_or_else(|_| "tests/fixtures/pdf".to_string())
}

/// Cartella base dei modelli PaddleOCR.
/// Configura con: $env:PPOCR_MODELS_DIR = "C:\path\to\models\paddleocr"
fn paddleocr_dir() -> std::path::PathBuf {
    std::env::var("PPOCR_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("models/paddleocr"))
}

// ─── Setup ───────────────────────────────────────────────────────────────────

fn setup_ocr() -> OcrLite {
    let hub = ModelHub::with_default_cache().expect("cache dir");
    let paths = hub.ensure(PpOcrVersion::V6Tiny).expect("modelli OCR");
    let ori_paths = hub.ensure_single(PpStructureModel::DocOrientation)
        .expect("DocOrientation model");
    let ori_clf = DocOrientationClassifier::from_path(&ori_paths.onnx)
        .expect("DocOrientationClassifier");

    let mut ocr = OcrLite::new();
    ocr.init_models_no_angle(
        paths.det_onnx.to_str().unwrap(),
        paths.rec_onnx.to_str().unwrap(),
        paths.dict_txt.to_str().unwrap(),
        4,
    )
    .expect("init_models_no_angle");
    ocr.set_doc_orientation_model(ori_clf);
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
    TableStructureRecognizer::from_path_with_dict(&model, Some(&dict))
        .expect("TableStructureRecognizer")
        .with_input_size(488)
}


// ─── PDF → PNG ───────────────────────────────────────────────────────────────

/// Converte un PDF in PNG a 200 DPI via pdftoppm.
/// Ritorna Vec di percorsi PNG nell'ordine delle pagine.
fn pdf_to_png(pdf_path: &Path, out_dir: &Path) -> Vec<PathBuf> {
    let stem = pdf_path.file_stem().unwrap().to_str().unwrap();
    let prefix = out_dir.join(stem);
    let prefix_str = prefix.to_str().unwrap();

    let status = Command::new("pdftoppm")
        .args(["-r", "200", "-png", pdf_path.to_str().unwrap(), prefix_str])
        .output()
        .expect("pdftoppm non trovato in PATH");

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        let relevant: String = err.lines()
            .filter(|l| !l.contains("Syntax Error") && !l.contains("MiKTeX"))
            .collect::<Vec<_>>().join("\n");
        if !relevant.is_empty() {
            eprintln!("  [pdftoppm] {relevant}");
        }
    }

    let mut pages: Vec<PathBuf> = std::fs::read_dir(out_dir)
        .expect("read_dir out")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str())
                .map(|n| n.starts_with(stem) && n.ends_with(".png"))
                .unwrap_or(false)
        })
        .collect();
    pages.sort();
    pages
}

// ─── Table helpers ───────────────────────────────────────────────────────────

fn crop_layout_box(img: &image::RgbImage, lb: &LayoutBox) -> image::RgbImage {
    let x = lb.x.min(img.width().saturating_sub(1));
    let y = lb.y.min(img.height().saturating_sub(1));
    let w = lb.w.min(img.width().saturating_sub(x));
    let h = lb.h.min(img.height().saturating_sub(y));
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

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
    parts.into_iter().map(|(_, t)| t).filter(|t| !t.is_empty())
        .collect::<Vec<_>>().join(" ")
}

fn table_to_markdown(
    structure: &TableStructure,
    ocr_result: &OcrResult,
    lb: &LayoutBox,
) -> String {
    let cells = &structure.cell_boxes;
    if cells.is_empty() {
        return format!("*(nessun cell_box)*");
    }
    let n_rows = structure.html_tokens.matches("<tr>").count().max(1);
    let n_cols = ((cells.len() + n_rows - 1) / n_rows).max(1);

    let mut ordered: Vec<(usize, f32, f32)> = cells.iter().enumerate()
        .map(|(i, c)| (i, (c.x1 + c.x2) * 0.5, (c.y1 + c.y2) * 0.5))
        .collect();
    ordered.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut rows: Vec<Vec<String>> = Vec::new();
    for chunk in ordered.chunks(n_cols) {
        let mut row = chunk.to_vec();
        row.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        rows.push(row.iter()
            .map(|&(i, _, _)| collect_cell_text(ocr_result, &cells[i], lb.x, lb.y))
            .collect());
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

// ─── Page runner ─────────────────────────────────────────────────────────────

struct PageResult {
    n_boxes:   usize,
    n_tables:  usize,
    n_chars:   usize,
    ms_orient: u64,
    ms_layout: u64,
    ms_ocr:    u64,
    ms_table:  u64,
}

fn run_page(
    ocr: &mut OcrLite,
    layout: &mut LayoutAnalyzer,
    recognizer: &mut TableStructureRecognizer,
    img: &image::RgbImage,
) -> PageResult {
    // ── Layout (su immagine raw — il classificatore orient non è ancora girato) ──
    // Nota: detect_with_options applica la rotazione internamente via
    // OcrOptions.use_doc_orientation; il layout però riceve l'immagine raw.
    // Per i test correnti (Art_*) non fa differenza: layout + OCR vengono
    // eseguiti nello stesso orientamento finale grazie alla pre-rotazione in OCR.
    let t0 = Instant::now();
    let layout_boxes = layout.analyze(img).expect("layout.analyze");
    let ms_layout = t0.elapsed().as_millis() as u64;

    // ── OCR con correzione orientamento integrata ───────────────────────────
    let t_ocr = Instant::now();
    let ocr_result = ocr.detect_with_options(
        img, 50, 960, 0.5, 0.3, 1.6,
        false, false,
        OcrOptions::default(),
    ).expect("detect_with_options");
    let ms_ocr = t_ocr.elapsed().as_millis() as u64;
    let ms_orient = 0u64; // incluso in ms_ocr

    if ocr_result.page_angle != 0 {
        eprintln!("  Orientamento corretto: {}° → OCR su immagine ruotata", ocr_result.page_angle);
    }

    // ── XY-Cut text ────────────────────────────────────────────────────────
    let lb_order = if layout_boxes.is_empty() { vec![] } else { xy_cut_order(&layout_boxes) };
    struct Block { lb: usize, cy: u32, text: String }
    let blocks: Vec<Block> = ocr_result.text_blocks.iter().map(|tb| {
        let pts = &tb.box_points;
        let cx = pts.iter().map(|p| p.x).sum::<u32>() / pts.len() as u32;
        let cy = pts.iter().map(|p| p.y).sum::<u32>() / pts.len() as u32;
        let lb = layout_boxes.iter().position(|lb| lb.contains(cx, cy))
            .unwrap_or_else(|| {
                layout_boxes.iter().enumerate()
                    .min_by(|(_, a), (_, b)| a.distance_to(cx, cy)
                        .partial_cmp(&b.distance_to(cx, cy))
                        .unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i).unwrap_or(0)
            });
        Block { lb, cy, text: tb.text.trim().to_string() }
    }).collect();

    let mut by_lb: std::collections::HashMap<usize, Vec<(u32, String)>> = std::collections::HashMap::new();
    for b in &blocks {
        if !b.text.is_empty() { by_lb.entry(b.lb).or_default().push((b.cy, b.text.clone())); }
    }
    for v in by_lb.values_mut() { v.sort_by_key(|(cy, _)| *cy); }

    let mut text_lines = Vec::new();
    if lb_order.is_empty() {
        let mut sorted = blocks;
        sorted.sort_by_key(|b| b.cy);
        for b in sorted { if !b.text.is_empty() { text_lines.push(b.text); } }
    } else {
        for lb_idx in &lb_order {
            if let Some(group) = by_lb.remove(lb_idx) {
                for (_, t) in group { text_lines.push(t); }
            }
        }
        for (_, group) in by_lb { for (_, t) in group { text_lines.push(t); } }
    }
    let text = text_lines.join("\n");

    // Stampa testo (prime 15 righe)
    eprintln!("  ── testo (XY-Cut) ──");
    for (i, line) in text.lines().take(15).enumerate() {
        eprintln!("  {:2}│ {line}", i + 1);
    }
    if text.lines().count() > 15 { eprintln!("  ... ({} righe)", text.lines().count()); }

    // ── Tabelle ─────────────────────────────────────────────────────────────
    let table_boxes: Vec<&LayoutBox> = layout_boxes.iter()
        .filter(|lb| lb.class == LayoutClass::Table)
        .collect();
    let n_tables = table_boxes.len();
    let mut ms_table = 0u64;

    for (ti, lb) in table_boxes.iter().enumerate() {
        eprintln!("  ── Tabella #{} (score={:.2} x={} y={} w={} h={}) ──",
            ti + 1, lb.score, lb.x, lb.y, lb.w, lb.h);
        let crop = crop_layout_box(img, lb);
        let t2 = Instant::now();
        match recognizer.recognize(&crop) {
            Ok(structure) => {
                ms_table += t2.elapsed().as_millis() as u64;
                eprintln!("  SLANet+: cells={} score={:.3}",
                    structure.cell_boxes.len(), structure.score);
                let md = table_to_markdown(&structure, &ocr_result, lb);
                for line in md.lines().take(12) { eprintln!("  {line}"); }
                if md.lines().count() > 12 { eprintln!("  ... ({} righe)", md.lines().count()); }
            }
            Err(e) => {
                ms_table += t2.elapsed().as_millis() as u64;
                eprintln!("  SLANet+ errore: {e}");
            }
        }
    }

    PageResult {
        n_boxes:   layout_boxes.len(),
        n_tables,
        n_chars:   text.len(),
        ms_orient,
        ms_layout,
        ms_ocr,
        ms_table,
    }
}

// ─── Test ─────────────────────────────────────────────────────────────────────

#[test]
fn art_pipeline_end_to_end() {
    let root_str = tests_root();
    let root = Path::new(&root_str);
    if !root.exists() {
        eprintln!("[skip] PPOCR_TEST_PDF_DIR non trovata: {} — imposta la var d'ambiente", root_str);
        return;
    }

    // Trova sotto-cartelle con PDF
    let art_dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .expect("read_dir tests root")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    if art_dirs.is_empty() {
        eprintln!("[skip] nessuna cartella in {root_str}");
        return;
    }

    // Directory temp per i PNG
    let tmp_root = root.join("_pdf_png_tmp");
    std::fs::create_dir_all(&tmp_root).expect("crea tmp png dir");

    let mut ocr      = setup_ocr();  // include DocOrientationClassifier via set_doc_orientation_model
    let mut layout   = setup_layout();
    let mut recognizer = setup_table_recognizer();

    // Contatori globali
    let mut total_pdfs   = 0usize;
    let mut total_pages  = 0usize;
    let mut total_tables = 0usize;
    let mut total_chars  = 0usize;
    let mut total_ms_orient  = 0u64;
    let mut total_ms_layout  = 0u64;
    let mut total_ms_ocr     = 0u64;
    let mut total_ms_table   = 0u64;
    let mut total_ms_pdf2img = 0u64;

    let t_global = Instant::now();

    for art_dir in &{
        let mut v = art_dirs;
        v.sort();
        v
    } {
        let dir_name = art_dir.file_name().unwrap().to_str().unwrap();
        eprintln!("\n╔══════════════════════════════════════════════╗");
        eprintln!("║  {dir_name:<44}║");
        eprintln!("╚══════════════════════════════════════════════╝");

        let pdf_files: Vec<PathBuf> = std::fs::read_dir(art_dir)
            .expect("read_dir Art dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("pdf")).unwrap_or(false))
            .collect::<Vec<_>>().into_iter().collect();

        let mut pdf_files = pdf_files;
        pdf_files.sort();

        for pdf_path in &pdf_files {
            total_pdfs += 1;
            let pdf_name = pdf_path.file_name().unwrap().to_str().unwrap();
            eprintln!("\n  PDF: {pdf_name}");

            // Prepara directory png per questo PDF
            let png_dir = tmp_root.join(dir_name).join(pdf_path.file_stem().unwrap());
            std::fs::create_dir_all(&png_dir).expect("crea png_dir");

            // Converti PDF → PNG
            let t_conv = Instant::now();
            let png_pages = pdf_to_png(pdf_path, &png_dir);
            let ms_conv = t_conv.elapsed().as_millis() as u64;
            total_ms_pdf2img += ms_conv;

            eprintln!("  {} pagine convertite in {ms_conv}ms ({} DPI)",
                png_pages.len(), 200);

            for (page_idx, png_path) in png_pages.iter().enumerate() {
                let page_num = page_idx + 1;
                let img = image::open(png_path)
                    .unwrap_or_else(|e| panic!("open PNG {:?}: {e}", png_path))
                    .to_rgb8();

                eprintln!("\n  ══ Pag {page_num}/{} — {}×{} ══",
                    png_pages.len(), img.width(), img.height());

                let res = run_page(&mut ocr, &mut layout, &mut recognizer, &img);

                eprintln!("  Tempi: orient={}ms  layout={}ms  ocr={}ms  table={}ms  totale={}ms",
                    res.ms_orient, res.ms_layout, res.ms_ocr, res.ms_table,
                    res.ms_orient + res.ms_layout + res.ms_ocr + res.ms_table);
                eprintln!("  Layout box: {}  Tabelle: {}  Chars OCR: {}",
                    res.n_boxes, res.n_tables, res.n_chars);

                total_pages  += 1;
                total_tables += res.n_tables;
                total_chars  += res.n_chars;
                total_ms_orient += res.ms_orient;
                total_ms_layout += res.ms_layout;
                total_ms_ocr    += res.ms_ocr;
                total_ms_table  += res.ms_table;
            }
        }
    }

    let total_wall = t_global.elapsed().as_millis() as u64;

    eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║  RIEPILOGO ESECUZIONE                                        ║");
    eprintln!("╠══════════════════════════════════════════════════════════════╣");
    eprintln!("║  PDF processati : {total_pdfs:<43}║");
    eprintln!("║  Pagine totali  : {total_pages:<43}║");
    eprintln!("║  Tabelle trovate: {total_tables:<43}║");
    eprintln!("║  Caratteri OCR  : {total_chars:<43}║");
    eprintln!("╠══════════════════════════════════════════════════════════════╣");
    eprintln!("║  TEMPI (totale e media/pagina)                               ║");
    let avg = |ms: u64| if total_pages > 0 { ms / total_pages as u64 } else { 0 };
    eprintln!("║  PDF→PNG  total={}ms  avg={}/pag{:>22}║",
        total_ms_pdf2img, avg(total_ms_pdf2img), "");
    eprintln!("║  Orient   total={}ms  avg={}/pag{:>22}║",
        total_ms_orient, avg(total_ms_orient), "");
    eprintln!("║  Layout   total={}ms  avg={}/pag{:>22}║",
        total_ms_layout, avg(total_ms_layout), "");
    eprintln!("║  OCR      total={}ms  avg={}/pag{:>22}║",
        total_ms_ocr, avg(total_ms_ocr), "");
    eprintln!("║  Table    total={}ms  avg={}/pag{:>22}║",
        total_ms_table, avg(total_ms_table), "");
    eprintln!("║  WALL CLOCK TOTALE: {}ms = {:.1}s{:>27}║",
        total_wall, total_wall as f64 / 1000.0, "");
    eprintln!("╚══════════════════════════════════════════════════════════════╝");

    assert!(total_pages >= 1, "nessuna pagina processata");
    assert!(total_chars >= 100, "troppo poco testo OCR ({total_chars} car) — pipeline rotta?");
}
