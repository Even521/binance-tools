use thiserror::Error;

#[derive(Error, Debug)]
pub enum OcrError {
    /// Da rc.13 `ort::Error` è generico su un tipo di *recovery*
    /// (`Error<R>`: i metodi del `SessionBuilder` ritornano
    /// `Error<SessionBuilder>`). Non essendo derivabile un `#[from]` per
    /// ogni `R`, la conversione è l'impl generica in fondo al file.
    #[error("Ort error [{code:?}]: {message}")]
    Ort { code: ort::error::ErrorCode, message: String },
    #[error("Io error")]
    Io(#[from] std::io::Error),
    #[error("Session not initialized")]
    ImageError(#[from] image::ImageError),
    #[error("Image error")]
    SessionNotInitialized,
    /// Errore nella shape o nei nomi degli input ONNX (modello incompatibile).
    #[error("Model input error: {0}")]
    ModelInput(String),
    /// Errore nella shape o decodifica dell'output ONNX.
    #[error("Model output error: {0}")]
    ModelOutput(String),
    /// Errore download / cache modelli (model_hub).
    #[error("Model hub error: {0}")]
    ModelHubError(String),
}

impl<R> From<ort::Error<R>> for OcrError {
    fn from(e: ort::Error<R>) -> Self {
        OcrError::Ort { code: e.code(), message: e.message().to_string() }
    }
}
