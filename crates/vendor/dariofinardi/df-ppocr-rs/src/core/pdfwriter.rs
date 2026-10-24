// Copyright (c) 2026 Dario Finardi / Jugaad s.r.l.
// https://omissis.ai
// All rights reserved. Distribuzione vietata senza autorizzazione scritta.

//! Scrittura di un PDF **ricercabile** multipagina: per ogni pagina
//! l'immagine rasterizzata più un layer di testo **invisibile** (`3 Tr`)
//! costruito dai [`TextItem`] di [`crate::pdf`].
//!
//! Modulo puro (niente `ort`, niente FFI): compila su wasm32, quindi il
//! PDF si genera interamente nel browser senza passare da un motore PDF
//! esterno.
//!
//! ## Scelte di formato, e perché
//!
//! - **Immagine `DCTDecode`**: i byte JPEG prodotti dal canvas entrano nel
//!   PDF così come sono, senza ricodifica. Zero perdita aggiuntiva e zero
//!   costo di conversione.
//! - **GlyphLessFont (Type0 / Identity-H)**: lo stesso font di
//!   TessPDFRenderer — glifi vuoti, `DW=500`, `Descent≈0`.
//!   Il box di selezione coincide col bbox OCR; niente correzioni
//!   empiriche su metriche Courier. Copertura Unicode completa via
//!   ToUnicode identity + stringhe hex UTF-16BE.
//! - **`Tz` per item**: il testo viene scalato orizzontalmente perché la
//!   sua estensione coincida con quella del box OCR. Con DW=500 la
//!   larghezza naturale è `0.5 · size · n_char` — esatta, senza tabelle.
//! - **Spazio in coda a ogni item**: separatore esplicito per gli
//!   estrattori, a costo zero perché il testo è invisibile. Il `Tz` è
//!   calcolato sulla parola *senza* lo spazio, così l'avanzamento in più
//!   cade esattamente nel vuoto fra una parola e la successiva.
//! - **`Tm` assoluto per ogni item** invece di `Td` relativo: niente
//!   accumulo di errore, e la rotazione entra nella stessa matrice.

use serde::{Deserialize, Serialize};

use crate::core::pdf::TextItem;

/// `pdf.ttf` di Tesseract (GlyphLessFont), ~572 byte.
const PDF_TTF: &[u8] = include_bytes!("../../assets/pdf.ttf");
/// CIDToGIDMap compresso (Flate): tutti i CID → GID 1.
const CIDTOGID_FLATE: &[u8] = include_bytes!("../../assets/cidtogidmap.flate");

/// Larghezza di ogni glifo GlyphLessFont, in millesimi del corpo (DW=500).
const GLYPHLESS_WIDTH_PER_MILLE: f32 = 500.0;

const TOUNICODE_CMAP: &[u8] = br"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo
<<
  /Registry (Adobe)
  /Ordering (UCS)
  /Supplement 0
>> def
/CMapName /Adobe-Identify-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginbfrange
<0000> <FFFF> <0000>
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end
";

/// Una pagina del PDF da produrre.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PdfPage {
    /// Offset e lunghezza dei byte JPEG dentro il blob passato a parte.
    /// Si evita così di attraversare il confine wasm con un array di
    /// numeri per ogni pagina.
    pub jpeg_offset: usize,
    pub jpeg_len: usize,
    /// Dimensioni della pagina PDF in punti (MediaBox).
    pub width_pt: f32,
    pub height_pt: f32,
    /// Testo invisibile, già in coordinate pagina (vedi `crate::pdf`).
    pub items: Vec<TextItem>,
}

#[derive(Debug)]
pub enum PdfError {
    NoPages,
    JpegOutOfRange { page: usize },
    EmptyJpeg { page: usize },
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfError::NoPages => write!(f, "nessuna pagina da scrivere"),
            PdfError::JpegOutOfRange { page } =>
                write!(f, "pagina {page}: offset/len del JPEG fuori dal blob"),
            PdfError::EmptyJpeg { page } => write!(f, "pagina {page}: JPEG vuoto"),
        }
    }
}

// ─── Codifica testo ───────────────────────────────────────────────────────────

/// Stringa PDF hex UTF-16BE per Identity-H (`<00410042>`).
fn pdf_hex_utf16be(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + s.len() * 4);
    out.push(b'<');
    for c in s.chars() {
        let cp = c as u32;
        if cp > 0xFFFF {
            let cp = cp - 0x10000;
            let hi = 0xD800 + (cp >> 10);
            let lo = 0xDC00 + (cp & 0x3FF);
            push_hex4(&mut out, hi as u16);
            push_hex4(&mut out, lo as u16);
        } else {
            push_hex4(&mut out, cp as u16);
        }
    }
    out.push(b'>');
    out
}

fn push_hex4(out: &mut Vec<u8>, v: u16) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push(HEX[((v >> 12) & 0xF) as usize]);
    out.push(HEX[((v >> 8) & 0xF) as usize]);
    out.push(HEX[((v >> 4) & 0xF) as usize]);
    out.push(HEX[(v & 0xF) as usize]);
}

/// Formatta un float in modo compatto e deterministico (niente notazione
/// esponenziale, che alcuni parser PDF rifiutano).
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    // "-0" è legale in PDF ma inutile e mal digerito da qualche parser:
    // normalizzalo, insieme ai casi degeneri di troncamento.
    if s.is_empty() || s == "-" || s == "-0" { "0".into() } else { s.to_string() }
}

// ─── Scrittura ────────────────────────────────────────────────────────────────

struct Writer {
    buf: Vec<u8>,
    offsets: Vec<usize>, // offsets[i] = byte offset dell'oggetto i+1
}

impl Writer {
    fn new() -> Self {
        Self { buf: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(), offsets: Vec::new() }
    }
    fn obj(&mut self, id: usize, body: &[u8]) {
        if self.offsets.len() < id {
            self.offsets.resize(id, 0);
        }
        self.offsets[id - 1] = self.buf.len();
        self.buf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        self.buf.extend_from_slice(body);
        self.buf.extend_from_slice(b"\nendobj\n");
    }
    fn stream(&mut self, id: usize, dict_extra: &str, data: &[u8]) {
        let head = format!("<< /Length {}{} >>\nstream\n", data.len(), dict_extra);
        let mut body = head.into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.obj(id, &body);
    }
    fn finish(mut self, root: usize) -> Vec<u8> {
        let n = self.offsets.len();
        let xref_at = self.buf.len();
        self.buf.extend_from_slice(format!("xref\n0 {}\n", n + 1).as_bytes());
        self.buf.extend_from_slice(b"0000000000 65535 f \n");
        for off in &self.offsets {
            self.buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        self.buf.extend_from_slice(
            format!("trailer\n<< /Size {} /Root {} 0 R >>\nstartxref\n{}\n%%EOF\n",
                    n + 1, root, xref_at).as_bytes());
        self.buf
    }
}

/// Limiti del `Tz` in percento: fuori da questo intervallo il box OCR è
/// degenere (larghezza nulla o enorme) e uno scaling estremo peggiora
/// l'estrazione invece di aiutarla.
const HSCALE_MIN: f32 = 10.0;
const HSCALE_MAX: f32 = 1000.0;

/// `Tz` che porta la larghezza naturale GlyphLessFont (DW=500) a
/// coincidere con quella del box OCR.
fn hscale_percent(it: &TextItem) -> f32 {
    let n = it.text.trim_end().chars().count();
    if n == 0 || it.width_pt <= 0.0 || it.font_size_pt <= 0.0 {
        return 100.0;
    }
    let natural = (GLYPHLESS_WIDTH_PER_MILLE / 1000.0) * it.font_size_pt * n as f32;
    (100.0 * it.width_pt / natural).clamp(HSCALE_MIN, HSCALE_MAX)
}

/// Content stream di una pagina: immagine a piena pagina, poi il testo
/// invisibile nell'ordine dato (che È il reading order).
fn page_content(p: &PdfPage) -> Vec<u8> {
    let mut s = Vec::new();
    // Immagine: la matrice `cm` la scala dall'unità alla pagina intera.
    s.extend_from_slice(
        format!("q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n",
                num(p.width_pt), num(p.height_pt)).as_bytes());

    let visible: Vec<&TextItem> = p.items.iter()
        .filter(|it| !it.text.trim().is_empty() && it.font_size_pt > 0.0)
        .collect();
    if visible.is_empty() {
        return s;
    }

    s.extend_from_slice(b"BT\n3 Tr\n");
    let mut cur_size = f32::NAN;
    let mut cur_hscale = f32::NAN;
    for it in visible {
        if it.font_size_pt != cur_size {
            s.extend_from_slice(format!("/F1 {} Tf\n", num(it.font_size_pt)).as_bytes());
            cur_size = it.font_size_pt;
        }
        // Tz: allinea l'estensione del testo al box OCR. Arrotondato a
        // 0.1% per non riemettere l'operatore a ogni minima variazione.
        let hs = (hscale_percent(it) * 10.0).round() / 10.0;
        if hs != cur_hscale {
            s.extend_from_slice(format!("{} Tz\n", num(hs)).as_bytes());
            cur_hscale = hs;
        }
        // Tm assoluto: include la rotazione della baseline.
        // Gli angoli sono solo multipli di 90° (dal classificatore di
        // orientamento a 4 classi): valori esatti invece di sin/cos, che
        // per 180° darebbero un seno di -8.7e-8 e quindi un "-0" nel file.
        let (a, b, c, d) = match it.rotation_deg % 360 {
            90  => (0.0, 1.0, -1.0, 0.0),
            180 => (-1.0, 0.0, 0.0, -1.0),
            270 => (0.0, -1.0, 1.0, 0.0),
            0   => (1.0, 0.0, 0.0, 1.0),
            other => {
                let r = (other as f32).to_radians();
                (r.cos(), r.sin(), -r.sin(), r.cos())
            }
        };
        s.extend_from_slice(format!("{} {} {} {} {} {} Tm\n",
            num(a), num(b), num(c), num(d), num(it.x_pt), num(it.y_pt)).as_bytes());
        // Spazio in coda = separatore ESPLICITO per gli estrattori: senza,
        // due parole vicine (o l'ultima di una riga e la prima della
        // successiva) vengono concatenate — visto sul campo:
        // "FINARDI"+"DARIO" → "FINARDIDARIO". Trim prima, per non
        // raddoppiare eventuali spazi già presenti nell'item.
        let mut text = it.text.trim_end().to_string();
        text.push(' ');
        s.extend_from_slice(&pdf_hex_utf16be(&text));
        s.extend_from_slice(b" Tj\n");
    }
    s.extend_from_slice(b"ET\n");
    s
}

/// Esito della scrittura, per diagnostica lato chiamante.
#[derive(Debug, Clone, Serialize)]
pub struct PdfBuildInfo {
    pub bytes: usize,
    pub pages: usize,
    pub text_items: usize,
    /// Storico WinAnsi: con GlyphLessFont/Identity-H resta sempre 0
    /// (copertura Unicode completa). Campo mantenuto per compatibilità API.
    pub unmapped_chars: usize,
}

/// Costruisce il PDF. `jpeg_blob` contiene i JPEG di tutte le pagine
/// concatenati; ogni [`PdfPage`] vi punta con offset e lunghezza.
pub fn build_pdf(pages: &[PdfPage], jpeg_blob: &[u8]) -> Result<(Vec<u8>, PdfBuildInfo), PdfError> {
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    for (i, p) in pages.iter().enumerate() {
        if p.jpeg_len == 0 {
            return Err(PdfError::EmptyJpeg { page: i + 1 });
        }
        if p.jpeg_offset + p.jpeg_len > jpeg_blob.len() {
            return Err(PdfError::JpegOutOfRange { page: i + 1 });
        }
    }

    // Layout oggetti (come TessPDFRenderer):
    // 1 Catalog · 2 Pages · 3 Type0 · 4 CIDFont · 5 CIDToGID · 6 ToUnicode
    // · 7 FontDescriptor · 8 FontFile2 · poi 3 oggetti per pagina.
    const CATALOG: usize = 1;
    const PAGES: usize = 2;
    const FONT: usize = 3; // Type0 /F1
    const CIDFONT: usize = 4;
    const CIDTOGID: usize = 5;
    const TOUNICODE: usize = 6;
    const FONTDESC: usize = 7;
    const FONTFILE: usize = 8;
    let page_obj = |i: usize| 9 + 3 * i;
    let content_obj = |i: usize| 10 + 3 * i;
    let image_obj = |i: usize| 11 + 3 * i;

    let mut w = Writer::new();
    let mut text_items = 0usize;

    w.obj(CATALOG, format!("<< /Type /Catalog /Pages {PAGES} 0 R >>").as_bytes());

    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", page_obj(i))).collect();
    w.obj(PAGES, format!("<< /Type /Pages /Count {} /Kids [{}] >>",
                         pages.len(), kids.join(" ")).as_bytes());

    // GlyphLessFont — Type0 / Identity-H (TessPDFRenderer).
    w.obj(FONT, format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont \
/Encoding /Identity-H /DescendantFonts [{CIDFONT} 0 R] /ToUnicode {TOUNICODE} 0 R >>"
    ).as_bytes());
    w.obj(CIDFONT, format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont \
/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
/FontDescriptor {FONTDESC} 0 R /DW 500 /CIDToGIDMap {CIDTOGID} 0 R >>"
    ).as_bytes());
    w.stream(CIDTOGID, " /Filter /FlateDecode", CIDTOGID_FLATE);
    w.stream(TOUNICODE, "", TOUNICODE_CMAP);
    w.obj(FONTDESC, format!(
        "<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 \
/Ascent 1000 /CapHeight 1000 /Descent -1 /ItalicAngle 0 /StemV 80 \
/FontBBox [0 0 500 1000] /FontFile2 {FONTFILE} 0 R >>"
    ).as_bytes());
    w.stream(FONTFILE, &format!(" /Length1 {}", PDF_TTF.len()), PDF_TTF);

    for (i, p) in pages.iter().enumerate() {
        w.obj(page_obj(i), format!(
            "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {} {}] \
/Resources << /XObject << /Im0 {} 0 R >> /Font << /F1 {FONT} 0 R >> >> \
/Contents {} 0 R >>",
            num(p.width_pt), num(p.height_pt), image_obj(i), content_obj(i),
        ).as_bytes());

        let content = page_content(p);
        text_items += p.items.iter().filter(|it| !it.text.trim().is_empty()).count();
        w.stream(content_obj(i), "", &content);

        let jpeg = &jpeg_blob[p.jpeg_offset..p.jpeg_offset + p.jpeg_len];
        let (px_w, px_h) = jpeg_size(jpeg).unwrap_or((1, 1));
        w.stream(image_obj(i), &format!(
            " /Type /XObject /Subtype /Image /Width {px_w} /Height {px_h} \
/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"), jpeg);
    }

    let bytes = w.finish(CATALOG);
    let info = PdfBuildInfo {
        bytes: bytes.len(), pages: pages.len(), text_items, unmapped_chars: 0,
    };
    Ok((bytes, info))
}

/// Larghezza e altezza di un JPEG, dai marker SOF.
///
/// Il PDF pretende `/Width` e `/Height` corretti nel dizionario
/// dell'XObject: dedurli dal file evita che il chiamante debba passarli e
/// che una svista li disallinei dai pixel reali.
pub fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 3 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // Marker senza payload.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = ((data[i + 2] as usize) << 8) | data[i + 3] as usize;
        // SOF0..SOF15, esclusi DHT(C4), JPG(C8) e DAC(CC) che non sono SOF.
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            if i + 9 < data.len() {
                let h = ((data[i + 5] as u32) << 8) | data[i + 6] as u32;
                let w = ((data[i + 7] as u32) << 8) | data[i + 8] as u32;
                return Some((w, h));
            }
            return None;
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, x: f32, y: f32, rot: u32) -> TextItem {
        TextItem {
            text: text.into(), x_pt: x, y_pt: y, width_pt: 50.0,
            font_size_pt: 10.0, rotation_deg: rot, confidence: 90.0, line_level: false,
        }
    }

    /// JPEG minimo valido: SOI + SOF0 con 40×30 + EOI.
    fn fake_jpeg() -> Vec<u8> {
        vec![
            0xFF, 0xD8,
            0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x1E, 0x00, 0x28,
            0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01,
            0xFF, 0xD9,
        ]
    }

    #[test]
    fn jpeg_size_reads_sof0() {
        assert_eq!(jpeg_size(&fake_jpeg()), Some((40, 30)));
        assert_eq!(jpeg_size(&[0, 1, 2, 3]), None);
    }

    #[test]
    fn pdf_hex_encodes_ascii_and_latin() {
        assert_eq!(pdf_hex_utf16be("A"), b"<0041>".to_vec());
        assert_eq!(pdf_hex_utf16be("à"), b"<00E0>".to_vec());
        assert_eq!(pdf_hex_utf16be("中"), b"<4E2D>".to_vec());
        // Spazio separatore in coda.
        assert_eq!(pdf_hex_utf16be("Ciao "), b"<004300690061006F0020>".to_vec());
    }

    #[test]
    fn num_is_compact_and_never_exponential() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1.500), "1.5");
        assert_eq!(num(0.0001), "0");
        assert_eq!(num(-2.25), "-2.25");
        assert_eq!(num(f32::NAN), "0");
        // -0 normalizzato: nasce dal seno di 180° in floating point.
        assert_eq!(num(-0.0000001), "0");
    }

    #[test]
    fn builds_a_two_page_pdf() {
        let j = fake_jpeg();
        let mut blob = j.clone();
        blob.extend_from_slice(&j);
        let pages = vec![
            PdfPage { jpeg_offset: 0, jpeg_len: j.len(), width_pt: 595.2, height_pt: 841.7,
                      items: vec![item("Ciao", 10.0, 800.0, 0)] },
            PdfPage { jpeg_offset: j.len(), jpeg_len: j.len(), width_pt: 595.2, height_pt: 841.7,
                      items: vec![item("Zweite Seite", 10.0, 800.0, 180)] },
        ];
        let (bytes, info) = build_pdf(&pages, &blob).unwrap();
        assert_eq!(info.pages, 2);
        assert_eq!(info.text_items, 2);
        assert_eq!(info.unmapped_chars, 0);

        let s = String::from_utf8_lossy(&bytes);
        assert!(s.starts_with("%PDF-1.7"));
        assert!(s.trim_end().ends_with("%%EOF"));
        assert!(s.contains("/Type /Catalog"));
        assert!(s.contains("/Count 2"));
        assert!(s.contains("3 Tr"), "il testo deve essere invisibile");
        // Spazio in coda come hex 0020.
        assert!(s.contains("<004300690061006F0020> Tj"), "atteso Ciao + spazio in hex");
        assert!(s.contains("/BaseFont /GlyphLessFont"));
        assert!(s.contains("/Encoding /Identity-H"));
        assert!(s.contains(" Tz\n"), "atteso horizontal scaling");
        assert!(s.contains("/Filter /DCTDecode"));
        assert!(s.contains("/MediaBox [0 0 595.2 841.7]"));
        // La seconda pagina è ruotata: matrice non identitaria.
        assert!(s.contains("-1 0 0 -1"), "rotazione 180 attesa nella Tm");
        // xref: 8 oggetti font/doc + 3×2 pagine = 14 → Size 15.
        assert!(s.contains("xref\n0 15\n"));
    }

    #[test]
    fn hscale_matches_box_width() {
        // "Ciao" (4 glifi) a corpo 10 in GlyphLessFont: naturale = 0.5·10·4 = 20 pt.
        // Box largo 40 pt → Tz 200%.
        let it = item("Ciao", 0.0, 0.0, 0);
        let it = TextItem { width_pt: 40.0, ..it };
        assert!((hscale_percent(&it) - 200.0).abs() < 0.01);
        // Lo spazio in coda NON entra nel conteggio: il Tz si calcola
        // sulla parola nuda.
        let it2 = TextItem { text: "Ciao ".into(), ..it };
        assert!((hscale_percent(&it2) - 200.0).abs() < 0.01);
        // Box degenere → clamp.
        let degenerate = TextItem { width_pt: 0.001, ..it2 };
        assert!((hscale_percent(&degenerate) - HSCALE_MIN).abs() < 0.01);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(build_pdf(&[], &[]), Err(PdfError::NoPages)));
        let p = vec![PdfPage { jpeg_offset: 0, jpeg_len: 100, width_pt: 10.0,
                               height_pt: 10.0, items: vec![] }];
        assert!(matches!(build_pdf(&p, &[1, 2, 3]), Err(PdfError::JpegOutOfRange { page: 1 })));
    }

    #[test]
    fn glyphless_assets_are_present() {
        assert!(PDF_TTF.len() > 100, "pdf.ttf mancante/corrotto");
        assert_eq!(&PDF_TTF[0..4], b"\x00\x01\x00\x00"); // TrueType scalare
        assert!(!CIDTOGID_FLATE.is_empty());
    }
}
