// Copyright (c) 2026 Dario Finardi / Jugaad s.r.l.
// https://omissis.ai
// All rights reserved. Distribuzione vietata senza autorizzazione scritta.

//! Dal risultato OCR al layer di testo PDF: conversione delle coordinate
//! bitmap (pixel, origine in alto a sinistra, eventualmente raddrizzate
//! dall'orientation) allo spazio pagina PDF (punti, origine in basso a
//! sinistra), con gestione di `page_angle`.
//!
//! Modulo **puro** — compila su wasm32. Il chiamante TS prende i
//! [`TextItem`] e li scrive con `PageContent_*` (rdsdk):
//!
//! ```text
//! textSetRenderMode(3)                      ← invisibile
//! gsSetMatrix(rot(item.rotation_deg) ∘ T)   ← solo se rotation_deg != 0
//! textSetFont(font, item.font_size_pt)
//! textMove(item.x_pt, item.y_pt)
//! textSetHScale(100 * item.width_pt / larghezza_naturale)
//! drawText(item.text)
//! ```
//!
//! ## Convenzione di rotazione
//!
//! `page_angle` segue la semantica del classificatore PP-LCNet doc-ori e
//! del path nativo (`OcrLite::rotate_to_upright`):
//!
//! > **originale = ruota_oraria(raddrizzata, page_angle)**
//!
//! cioè `page_angle` è di quanto il contenuto *appare* ruotato in senso
//! orario nella pagina originale; il preprocessing raddrizza ruotando in
//! senso antiorario di `page_angle`, e l'OCR lavora sul bitmap
//! raddrizzato (`Page.width_px/height_px` sono le SUE dimensioni).
//! Questo modulo riporta le coordinate nello spazio della pagina PDF
//! originale, che non è stata toccata.

use serde::{Deserialize, Serialize};

use crate::core::types::Page;

/// Un item di testo pronto per `PageContent_*`, in coordinate pagina PDF
/// (punti, origine in basso a sinistra).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextItem {
    pub text: String,
    /// Origine della baseline (angolo di partenza della scrittura) in pt.
    pub x_pt: f32,
    pub y_pt: f32,
    /// Estensione del box lungo la direzione di scrittura, in pt. Serve al
    /// chiamante per calcolare l'horizontal scaling (`Tz`).
    pub width_pt: f32,
    /// Altezza del box = font size da passare a `textSetFont`, in pt.
    pub font_size_pt: f32,
    /// Rotazione della baseline in gradi **antiorari** (convenzione
    /// matrici PDF): `(360 − page_angle) % 360`. `0` per pagine dritte.
    pub rotation_deg: u32,
    /// Confidence OCR (0–100) dell'elemento — il chiamante può filtrare.
    pub confidence: f32,
    /// `true` se l'item è una linea intera (l'engine non aveva word box);
    /// `false` se è una singola parola.
    pub line_level: bool,
}

/// Mappa un punto dallo spazio del bitmap raddrizzato `(wu, hu)` allo
/// spazio del bitmap originale, invertendo la rotazione di `page_angle`.
/// Ritorna anche le dimensioni del bitmap originale.
fn upright_to_original(x: f32, y: f32, wu: f32, hu: f32, page_angle: u32) -> (f32, f32, f32, f32) {
    match page_angle % 360 {
        // originale = raddrizzata (nessuna rotazione)
        0 => (x, y, wu, hu),
        // originale = ruota_cw(raddrizzata, 90): (x,y) → (hu − y, x)
        90 => (hu - y, x, hu, wu),
        // 180: (x,y) → (wu − x, hu − y)
        180 => (wu - x, hu - y, wu, hu),
        // 270 cw = 90 ccw: (x,y) → (y, wu − x)
        270 => (y, wu - x, hu, wu),
        // Angoli non canonici non esistono per il classificatore 4-classi.
        _ => (x, y, wu, hu),
    }
}

/// Converte una [`Page`] OCR nei [`TextItem`] del layer di testo.
///
/// - `page_w_pt` / `page_h_pt`: MediaBox della pagina PDF **originale**
///   (quella che rdsdk sta mostrando), in punti.
/// - Cammina i blocchi nell'ordine in cui stanno in `page.blocks` — che È
///   il reading order — e dentro ogni linea emette le word; se una linea
///   non ha word (engine line-level) emette la linea intera.
///
/// Ritorna `Vec` vuoto se la pagina non ha dimensioni (`width_px == 0`).
pub fn page_to_text_items(page: &Page, page_w_pt: f32, page_h_pt: f32) -> Vec<TextItem> {
    if page.width_px == 0 || page.height_px == 0 || page_w_pt <= 0.0 || page_h_pt <= 0.0 {
        return Vec::new();
    }
    let (wu, hu) = (page.width_px as f32, page.height_px as f32);
    let angle = page.page_angle % 360;
    // Dimensioni del bitmap originale (per 90/270 gli assi si scambiano).
    let (w0, h0) = if angle == 90 || angle == 270 { (hu, wu) } else { (wu, hu) };
    let sx = page_w_pt / w0;
    let sy = page_h_pt / h0;
    let rotation_deg = (360 - angle) % 360;

    let mut items = Vec::new();
    let mut push = |text: &str, bbox: crate::core::types::BoundingBox, confidence: f32, line_level: bool| {
        if text.trim().is_empty() || bbox.is_empty() {
            return;
        }
        // Origine della scrittura = angolo in basso a sinistra del box
        // NELLO SPAZIO DI LETTURA (raddrizzato), poi riportato all'originale.
        let (x0, y0, _, _) = upright_to_original(
            bbox.left as f32, bbox.bottom as f32, wu, hu, angle,
        );
        let x_pt = x0 * sx;
        let y_pt = page_h_pt - y0 * sy;
        // Larghezza lungo la baseline e altezza del box: per 90/270 la
        // direzione di scrittura giace sull'asse y dell'originale, quindi
        // le scale si scambiano.
        let (width_pt, font_size_pt) = if angle == 90 || angle == 270 {
            (bbox.width() as f32 * sy, bbox.height() as f32 * sx)
        } else {
            (bbox.width() as f32 * sx, bbox.height() as f32 * sy)
        };
        items.push(TextItem {
            text: text.to_string(),
            x_pt, y_pt, width_pt, font_size_pt,
            rotation_deg,
            confidence,
            line_level,
        });
    };

    for block in &page.blocks {
        for para in &block.paragraphs {
            for line in &para.lines {
                if line.words.is_empty() {
                    push(&line.text, line.bbox, line.confidence, true);
                } else {
                    for w in &line.words {
                        push(&w.text, w.bbox, w.confidence, false);
                    }
                }
            }
        }
    }
    items
}

// ─── Filtro del rumore ────────────────────────────────────────────────────────

/// Criteri per scartare dal layer gli item che non sono testo.
///
/// Serve perché il rumore di scansione — bordo nero del piano, graffi,
/// puntinatura — viene riconosciuto come caratteri e finisce nel PDF: la
/// ricerca trova spazzatura e la selezione evidenzia il vuoto. È un
/// **palliativo**: la soluzione vera è ripulire il bitmap prima dell'OCR
/// (deskew, despeckle, rimozione bordi), vedi `docs/pipeline.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseFilter {
    /// Scarta sotto questa confidence (0–100). Il rumore esce quasi sempre
    /// molto basso; il testo stampato pulito sta sopra 70.
    pub min_confidence: f32,
    /// Scarta i token di 1 carattere che sono **tipici artefatti di
    /// scansione** (`|` `_` `~` `^` `\` `` ` ``), preservando la
    /// punteggiatura vera (`.` `,` `:` `;` `!` `?` `(` `)` `-` `'` `"`
    /// `%` `€` `/`).
    ///
    /// Distinzione necessaria: un filtro su "qualunque carattere non
    /// alfanumerico isolato" cancella anche i due punti di «Oggetto :» e
    /// la virgola di «rep. ,» — cioè contenuto reale.
    pub drop_single_punct: bool,
    /// Scarta i box più stretti o più bassi di così, in punti.
    pub min_size_pt: f32,
    /// Scarta gli item che iniziano entro questa distanza dal bordo
    /// sinistro. `0` = disattivato: alzarlo elimina il rumore del bordo di
    /// scansione ma può mangiare numerazioni e testo a margine.
    pub left_margin_pt: f32,
}

impl Default for NoiseFilter {
    fn default() -> Self {
        Self {
            min_confidence: 30.0,
            drop_single_punct: true,
            min_size_pt: 1.5,
            left_margin_pt: 0.0,
        }
    }
}

/// Statistiche dello scarto, per capire *cosa* è stato tolto invece di
/// fidarsi e basta.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FilterStats {
    pub kept: usize,
    pub dropped_confidence: usize,
    pub dropped_punct: usize,
    pub dropped_size: usize,
    pub dropped_margin: usize,
}

/// Caratteri che, **isolati**, sono quasi sempre rumore di scansione e non
/// contenuto: tratti verticali del bordo, graffi, sottolineature spezzate.
/// Tutto il resto della punteggiatura è contenuto e va tenuto.
fn is_scan_artifact(c: char) -> bool {
    matches!(c, '|' | '_' | '~' | '^' | '\\' | '`' | '¦' | '‗' | '·' | '•')
}

/// Applica il filtro, restituendo item superstiti e statistiche.
pub fn filter_noise(items: &[TextItem], f: &NoiseFilter) -> (Vec<TextItem>, FilterStats) {
    let mut st = FilterStats::default();
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let t = it.text.trim();
        if t.is_empty() {
            st.dropped_size += 1;
            continue;
        }
        if it.confidence < f.min_confidence {
            st.dropped_confidence += 1;
            continue;
        }
        if f.drop_single_punct {
            let mut ch = t.chars();
            if let (Some(c), None) = (ch.next(), ch.next()) {
                if is_scan_artifact(c) {
                    st.dropped_punct += 1;
                    continue;
                }
            }
        }
        if it.width_pt < f.min_size_pt || it.font_size_pt < f.min_size_pt {
            st.dropped_size += 1;
            continue;
        }
        if f.left_margin_pt > 0.0 && it.x_pt < f.left_margin_pt {
            st.dropped_margin += 1;
            continue;
        }
        st.kept += 1;
        out.push(it.clone());
    }
    (out, st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Block, BoundingBox, Line, Page, Paragraph, Word};

    fn page_with_word(wu: u32, hu: u32, angle: u32, bbox: BoundingBox) -> Page {
        let word = Word { text: "ciao".into(), bbox, confidence: 91.0 };
        let line = Line { text: "ciao".into(), bbox, confidence: 90.0, words: vec![word] };
        let para = Paragraph { bbox, lines: vec![line] };
        let block = Block { bbox, semantic_class: None, table_html: None, paragraphs: vec![para] };
        Page { page_number: 1, width_px: wu, height_px: hu, page_angle: angle, blocks: vec![block] }
    }

    // Bitmap 1000×1400 px → pagina 500×700 pt: scala 0.5 su entrambi gli assi.
    const W_PT: f32 = 500.0;
    const H_PT: f32 = 700.0;

    #[test]
    fn angle_0_flips_y_and_scales() {
        let page = page_with_word(1000, 1400, 0, BoundingBox::from_lrtb(100, 50, 300, 100));
        let items = page_to_text_items(&page, W_PT, H_PT);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        // Origine = (left, bottom) = (100, 100) px → (50, 700−50) pt.
        assert!((it.x_pt - 50.0).abs() < 1e-3);
        assert!((it.y_pt - 650.0).abs() < 1e-3);
        assert!((it.width_pt - 100.0).abs() < 1e-3);   // 200 px × 0.5
        assert!((it.font_size_pt - 25.0).abs() < 1e-3); // 50 px × 0.5
        assert_eq!(it.rotation_deg, 0);
        assert!(!it.line_level);
    }

    #[test]
    fn angle_90_maps_into_portrait_page() {
        // Originale portrait 1000×1400; contenuto ruotato 90° cw →
        // raddrizzato è landscape 1400×1000.
        let page = page_with_word(1400, 1000, 90, BoundingBox::from_lrtb(100, 50, 300, 100));
        let items = page_to_text_items(&page, W_PT, H_PT);
        let it = &items[0];
        // (l, b) = (100, 100) raddrizzato → originale (hu−y, x) = (900, 100)
        // → pt: x = 900×0.5 = 450, y = 700 − 100×0.5 = 650.
        assert!((it.x_pt - 450.0).abs() < 1e-3);
        assert!((it.y_pt - 650.0).abs() < 1e-3);
        // Baseline lungo l'asse y dell'originale → scala sy; altezza → sx.
        assert!((it.width_pt - 100.0).abs() < 1e-3);
        assert!((it.font_size_pt - 25.0).abs() < 1e-3);
        assert_eq!(it.rotation_deg, 270);
    }

    #[test]
    fn angle_180_mirrors_both_axes() {
        let page = page_with_word(1000, 1400, 180, BoundingBox::from_lrtb(100, 50, 300, 100));
        let it = &page_to_text_items(&page, W_PT, H_PT)[0];
        // (100, 100) → (900, 1300) px → (450, 700 − 650 = 50) pt.
        assert!((it.x_pt - 450.0).abs() < 1e-3);
        assert!((it.y_pt - 50.0).abs() < 1e-3);
        assert_eq!(it.rotation_deg, 180);
    }

    #[test]
    fn angle_270_maps_into_portrait_page() {
        let page = page_with_word(1400, 1000, 270, BoundingBox::from_lrtb(100, 50, 300, 100));
        let it = &page_to_text_items(&page, W_PT, H_PT)[0];
        // (100, 100) → (y, wu−x) = (100, 1300) px → (50, 700−650 = 50) pt.
        assert!((it.x_pt - 50.0).abs() < 1e-3);
        assert!((it.y_pt - 50.0).abs() < 1e-3);
        assert_eq!(it.rotation_deg, 90);
    }

    #[test]
    fn line_level_fallback_when_no_words() {
        let bbox = BoundingBox::from_lrtb(0, 0, 400, 40);
        let line = Line { text: "sola linea".into(), bbox, confidence: 80.0, words: Vec::new() };
        let para = Paragraph { bbox, lines: vec![line] };
        let block = Block { bbox, semantic_class: None, table_html: None, paragraphs: vec![para] };
        let page = Page { page_number: 1, width_px: 800, height_px: 600, page_angle: 0, blocks: vec![block] };
        let items = page_to_text_items(&page, 400.0, 300.0);
        assert_eq!(items.len(), 1);
        assert!(items[0].line_level);
        assert_eq!(items[0].text, "sola linea");
    }

    fn ti(text: &str, conf: f32, x: f32, w: f32, size: f32) -> TextItem {
        TextItem {
            text: text.into(), x_pt: x, y_pt: 100.0, width_pt: w,
            font_size_pt: size, rotation_deg: 0, confidence: conf, line_level: false,
        }
    }

    #[test]
    fn noise_filter_drops_low_confidence_and_artifacts() {
        let items = vec![
            ti("Banca", 92.0, 100.0, 40.0, 11.0),   // testo buono
            ti("|", 88.0, 12.0, 2.0, 6.0),          // artefatto bordo (conf ALTA)
            ti("x", 8.0, 300.0, 3.0, 9.0),          // confidence bassissima
            ti("mutuo", 71.0, 120.0, 35.0, 11.0),   // testo buono
        ];
        let (kept, st) = filter_noise(&items, &NoiseFilter::default());
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].text, "Banca");
        assert_eq!(kept[1].text, "mutuo");
        assert_eq!(st.dropped_confidence, 1);
        assert_eq!(st.dropped_punct, 1);
        assert_eq!(st.kept, 2);
    }

    /// Regressione osservata sul campo: «Oggetto :» diventava «Oggetto» e
    /// «rep. ,» perdeva la virgola. La punteggiatura vera è contenuto.
    #[test]
    fn noise_filter_preserves_real_punctuation() {
        let items = vec![
            ti(":", 90.0, 100.0, 3.0, 11.0),
            ti(",", 90.0, 110.0, 3.0, 11.0),
            ti(".", 90.0, 120.0, 3.0, 11.0),
            ti("(", 90.0, 130.0, 3.0, 11.0),
            ti("%", 90.0, 140.0, 6.0, 11.0),
            ti("€", 90.0, 150.0, 6.0, 11.0),
            ti("-", 90.0, 160.0, 4.0, 11.0),
        ];
        let (kept, st) = filter_noise(&items, &NoiseFilter::default());
        assert_eq!(kept.len(), items.len(), "nessuna punteggiatura va persa");
        assert_eq!(st.dropped_punct, 0);
    }

    #[test]
    fn left_margin_is_opt_in() {
        let items = vec![ti("37", 90.0, 8.0, 12.0, 10.0)];
        // Disattivato di default: un numero a margine NON va perso.
        let (kept, _) = filter_noise(&items, &NoiseFilter::default());
        assert_eq!(kept.len(), 1);
        // Attivato esplicitamente: viene scartato.
        let f = NoiseFilter { left_margin_pt: 20.0, ..Default::default() };
        let (kept, st) = filter_noise(&items, &f);
        assert!(kept.is_empty());
        assert_eq!(st.dropped_margin, 1);
    }

    #[test]
    fn noise_filter_keeps_single_digits_and_letters() {
        // "1" e "A" sono testo legittimo: solo la punteggiatura isolata cade.
        let items = vec![ti("1", 88.0, 50.0, 6.0, 10.0), ti("A", 88.0, 60.0, 8.0, 10.0)];
        let (kept, _) = filter_noise(&items, &NoiseFilter::default());
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn empty_or_degenerate_input_yields_nothing() {
        let page = page_with_word(0, 0, 0, BoundingBox::from_lrtb(0, 0, 10, 10));
        assert!(page_to_text_items(&page, W_PT, H_PT).is_empty());

        // Testo vuoto o bbox degenere vengono saltati, non emessi.
        let mut page = page_with_word(1000, 1400, 0, BoundingBox::default());
        page.blocks[0].paragraphs[0].lines[0].words[0].text = "  ".into();
        assert!(page_to_text_items(&page, W_PT, H_PT).is_empty());
    }
}
