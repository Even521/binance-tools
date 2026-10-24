// Copyright (c) 2026 Dario Finardi / Jugaad s.r.l.
// All rights reserved.

//! Nucleo **puro** della pipeline: nessun `ort`, nessuna FFI, nessun
//! `image`. Dipende solo da `serde` e compila su `wasm32-unknown-unknown`.
//!
//! ## Perché esiste
//!
//! La stessa logica serve a due esecuzioni molto diverse:
//!
//! - **nativa** — `ort` carica i modelli ONNX, questo crate orchestra;
//! - **browser** — i modelli girano su `onnxruntime-web` e il
//!   riconoscimento su `tesseract.js`, ma decodifica del layout, regole
//!   di lettura e costruzione della gerarchia devono dare **lo stesso
//!   risultato**.
//!
//! Tenere due copie di quelle regole significa vederle divergere: una
//! soglia corretta di qua e non di là, e i due prodotti smettono di
//! leggere la stessa pagina allo stesso modo. Qui la copia è una sola.
//!
//! ## Cosa contiene
//!
//! - [`types`]      — gerarchia `Page → Block → Paragraph → Line → Word`,
//!                    engine-agnostica: la riempie Paddle come Tesseract.
//! - [`layout`]     — `LayoutModelSpec` (parametri di preprocess presi dal
//!                    loro `inference.yml`, non dedotti dal grafo),
//!                    decodifica output, NMS, XY-Cut, rilevamento colonne,
//!                    associazione righe↔regioni, regole heading e
//!                    ocr-skip, costruzione della `Page`.
//! - [`preprocess`] — preparazione del bitmap indipendente dal backend.
//! - [`pdf`]        — coordinate bitmap → spazio pagina PDF, con
//!                    `page_angle` dal classificatore di orientamento.
//! - [`pdfwriter`]  — scrittura del text layer.
//!
//! ## Il vincolo da rispettare
//!
//! **Niente qui dentro può dipendere da `ort`, `image` o da una FFI.**
//! Se serve un'immagine, si passa un buffer e le sue dimensioni. La
//! verifica non è stilistica: `cargo check --no-default-features
//! --target wasm32-unknown-unknown` fallisce se la regola viene violata.

pub mod types;
pub mod layout;
pub mod preprocess;
pub mod pdf;
pub mod pdfwriter;

pub use types::{BoundingBox, Block, Hierarchy, Line, Page, Paragraph, Word};
