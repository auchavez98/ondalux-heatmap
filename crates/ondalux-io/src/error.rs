use std::fmt;

#[derive(Debug)]
pub enum IoError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Zip(zip::result::ZipError),
    Image(image::ImageError),
    /// The file is not a valid Ondalux Heatmap file.
    Format(String),
    /// The file was written by a newer Ondalux Heatmap version.
    NewerVersion {
        found: u32,
        supported: u32,
    },
    /// The input format is recognized but not supported.
    Unsupported(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Io(e) => write!(f, "I/O error: {e}"),
            IoError::Json(e) => write!(f, "invalid project data: {e}"),
            IoError::Zip(e) => write!(f, "invalid project archive: {e}"),
            IoError::Image(e) => write!(f, "image error: {e}"),
            IoError::Format(m) => write!(f, "invalid file: {m}"),
            IoError::NewerVersion { found, supported } => write!(
                f,
                "project format version {found} is newer than the supported version {supported}; update Ondalux Heatmap"
            ),
            IoError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IoError::Io(e) => Some(e),
            IoError::Json(e) => Some(e),
            IoError::Zip(e) => Some(e),
            IoError::Image(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::Io(e)
    }
}

impl From<serde_json::Error> for IoError {
    fn from(e: serde_json::Error) -> Self {
        IoError::Json(e)
    }
}

impl From<zip::result::ZipError> for IoError {
    fn from(e: zip::result::ZipError) -> Self {
        IoError::Zip(e)
    }
}

impl From<image::ImageError> for IoError {
    fn from(e: image::ImageError) -> Self {
        IoError::Image(e)
    }
}
