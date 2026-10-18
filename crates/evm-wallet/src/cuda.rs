use crate::VanityPattern;
use std::{error::Error, fmt, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CudaError {
    #[allow(dead_code)]
    Unavailable,
    #[allow(dead_code)]
    Runtime(i32),
}

impl fmt::Display for CudaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("CUDA backend is not available in this build"),
            Self::Runtime(code) => write!(f, "CUDA runtime operation failed ({code})"),
        }
    }
}

impl Error for CudaError {}

#[cfg(feature = "cuda")]
unsafe extern "C" {
    fn evm_wallet_cuda_available() -> i32;
    fn evm_wallet_cuda_max_batch() -> u32;
    fn evm_wallet_cuda_device_name(buffer: *mut u8, buffer_len: usize) -> i32;
    fn evm_wallet_cuda_search(
        parent_pubkey: *const u8,
        chain_code: *const u8,
        target: *const u8,
        target_len: usize,
        mode: u32,
        start_index: u32,
        count: u32,
        out_index: *mut u32,
    ) -> i32;
}

#[derive(Clone, Debug)]
struct CudaStatus {
    available: bool,
    reason: Option<&'static str>,
    device_name: Option<String>,
}

fn cuda_status() -> &'static CudaStatus {
    static STATUS: OnceLock<CudaStatus> = OnceLock::new();
    STATUS.get_or_init(|| {
        #[cfg(not(feature = "cuda"))]
        {
            CudaStatus {
                available: false,
                reason: Some("当前程序未包含 CUDA 后端"),
                device_name: None,
            }
        }
        #[cfg(feature = "cuda")]
        {
            // SAFETY: the bridge takes no pointers and returns a status code.
            if unsafe { evm_wallet_cuda_available() } != 1 {
                return CudaStatus {
                    available: false,
                    reason: Some("没有可用的 CUDA 设备"),
                    device_name: None,
                };
            }
            let mut buffer = [0_u8; 128];
            // SAFETY: the buffer is writable and remains alive for the call.
            let named = unsafe { evm_wallet_cuda_device_name(buffer.as_mut_ptr(), buffer.len()) };
            let device_name = if named == 0 {
                let end = buffer
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(buffer.len());
                String::from_utf8(buffer[..end].to_vec()).ok()
            } else {
                None
            };
            CudaStatus {
                available: true,
                reason: None,
                device_name,
            }
        }
    })
}

pub fn is_cuda_available() -> bool {
    cuda_status().available
}

pub fn cuda_unavailable_reason() -> Option<&'static str> {
    cuda_status().reason
}

pub fn cuda_device_name() -> Option<&'static str> {
    cuda_status().device_name.as_deref()
}

pub(crate) fn max_batch() -> u32 {
    #[cfg(feature = "cuda")]
    {
        // SAFETY: the bridge takes no pointers and returns a batch limit.
        let batch = unsafe { evm_wallet_cuda_max_batch() };
        if batch == 0 { 1 } else { batch }
    }
    #[cfg(not(feature = "cuda"))]
    {
        1
    }
}

pub(crate) fn search_xpub_batch(
    parent_pubkey: &[u8; 64],
    chain_code: &[u8; 32],
    pattern: &VanityPattern,
    start_index: u32,
    count: u32,
) -> Result<Option<u32>, CudaError> {
    #[cfg(feature = "cuda")]
    {
        let target = pattern.target().as_bytes();
        let mut index = 0;
        // SAFETY: all buffers remain alive for the duration of the synchronous FFI call.
        let status = unsafe {
            evm_wallet_cuda_search(
                parent_pubkey.as_ptr(),
                chain_code.as_ptr(),
                target.as_ptr(),
                target.len(),
                pattern.mode_code(),
                start_index,
                count,
                &mut index,
            )
        };
        match status {
            1 => Ok(Some(index)),
            0 => Ok(None),
            code => Err(CudaError::Runtime(code)),
        }
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = (parent_pubkey, chain_code, pattern, start_index, count);
        Err(CudaError::Unavailable)
    }
}
