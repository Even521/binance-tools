use anyhow::Result;
use std::path::Path;

use super::ocr::{
    contains_already_sent_confirmation, contains_rate_limit, contains_retryable_send_error,
    contains_sent_confirmation, recognize_document,
};

#[derive(Debug, Default)]
pub(super) struct SendStatus {
    pub already_sent: bool,
    pub sent: bool,
    pub rate_limited: bool,
    pub retryable_error: bool,
}

/// 识别发送结果。
/// 同一张截图只执行一次 OCR，然后判断所有发送状态。
pub(super) fn recognize_send_status(path: &Path) -> Result<SendStatus> {
    let doc = recognize_document(path)?;

    Ok(SendStatus {
        already_sent: contains_already_sent_confirmation(&doc.text),
        sent: contains_sent_confirmation(&doc.text),
        rate_limited: contains_rate_limit(&doc.text),
        retryable_error: contains_retryable_send_error(&doc.text)
            || doc
                .lines
                .iter()
                .any(|line| contains_retryable_send_error(&line.text)),
    })
}
