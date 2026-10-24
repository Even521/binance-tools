//! Punto unico di contatto con l'API di estrazione tensori di `ort`.
//!
//! rc.9 → rc.13: `try_extract_tensor::<T>()` ritornava `Result<ArrayViewD<T>>`,
//! ora ritorna `Result<(&Shape, &[T])>` — shape e slice raw, zero-copy;
//! l'`ArrayViewD` di prima corrisponde a `try_extract_array::<T>()`.
//! Entrambi i consumer qui vogliono un `Vec<f32>` piatto, quindi usiamo
//! direttamente la nuova firma.

use ort::value::Value;
use crate::ocr_error::OcrError;

/// Estrae un tensore f32 come `Vec<f32>`, scartando la shape.
pub fn tensor_to_vec_f32(value: &Value) -> Result<Vec<f32>, OcrError> {
    let (_shape, data) = value.try_extract_tensor::<f32>()?;
    Ok(data.to_vec())
}

/// Estrae un tensore f32 con la shape (per output di shape variabile).
///
/// Ritorna `(shape, data)` con shape già copiata in `Vec<i64>`.
pub fn tensor_extract_with_shape_f32(value: &Value) -> Result<(Vec<i64>, Vec<f32>), OcrError> {
    let (shape, data) = value.try_extract_tensor::<f32>()?;
    let shape: Vec<i64> = shape.iter().map(|&d| d as i64).collect();
    Ok((shape, data.to_vec()))
}
