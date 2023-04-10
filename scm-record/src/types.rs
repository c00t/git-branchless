use std::fmt::Display;
use std::num::TryFromIntError;
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
    #[error("failed to serialize JSON: {0}")]
    SerializeJson(#[source] serde_json::Error),

    #[error("failed to wrote file: {0}")]
    WriteFile(#[source] io::Error),

/// The Unix file mode. The special mode `0` indicates that the file did not exist.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FileMode(pub usize);

impl Display for FileMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(mode) = self;
        write!(f, "{mode:o}")
    }
}

impl From<usize> for FileMode {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<FileMode> for usize {
    fn from(value: FileMode) -> Self {
        let FileMode(value) = value;
        value
    }
}

impl TryFrom<u32> for FileMode {
    type Error = TryFromIntError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

impl TryFrom<FileMode> for u32 {
    type Error = TryFromIntError;

    fn try_from(value: FileMode) -> Result<Self, Self::Error> {
        let FileMode(value) = value;
        value.try_into()
    }
}

impl TryFrom<i32> for FileMode {
    type Error = TryFromIntError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

impl TryFrom<FileMode> for i32 {
    type Error = TryFromIntError;

    fn try_from(value: FileMode) -> Result<Self, Self::Error> {
        let FileMode(value) = value;
        value.try_into()
    }
}
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
                Section::FileMode { .. } => {}
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]