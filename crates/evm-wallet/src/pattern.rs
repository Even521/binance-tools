use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    Prefix,
    Suffix,
    Contains,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VanityPattern {
    mode: MatchMode,
    target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    Empty,
    TooLong,
    InvalidHex,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("请输入要匹配的地址字符"),
            Self::TooLong => f.write_str("匹配字符不能超过 40 位"),
            Self::InvalidHex => f.write_str("匹配字符只能使用十六进制字符 0-9、a-f"),
        }
    }
}

impl std::error::Error for PatternError {}

impl VanityPattern {
    #[cfg(feature = "cuda")]
    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    #[cfg(feature = "cuda")]
    pub(crate) fn mode_code(&self) -> u32 {
        match self.mode {
            MatchMode::Prefix => 0,
            MatchMode::Suffix => 1,
            MatchMode::Contains => 2,
        }
    }

    pub fn new(mode: MatchMode, target: &str) -> Result<Self, PatternError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(PatternError::Empty);
        }
        if target.len() > 40 {
            return Err(PatternError::TooLong);
        }
        if !target.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PatternError::InvalidHex);
        }

        Ok(Self {
            mode,
            target: target.to_ascii_lowercase(),
        })
    }

    /// Matches the 40 hexadecimal characters after the address's `0x` prefix.
    pub fn matches(&self, address: &str) -> bool {
        let address = address.strip_prefix("0x").unwrap_or(address);
        let address = address.to_ascii_lowercase();

        match self.mode {
            MatchMode::Prefix => address.starts_with(&self.target),
            MatchMode::Suffix => address.ends_with(&self.target),
            MatchMode::Contains => address.contains(&self.target),
        }
    }
}
