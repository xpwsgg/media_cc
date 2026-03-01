use std::path::PathBuf;
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "webp", "tiff", "tif", "raw", "cr2", "nef", "arw",
];

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "wmv", "flv", "webm", "m4v",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaFile {
    pub path: PathBuf,
    pub is_video: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanOutput {
    pub files: Vec<MediaFile>,
    pub errors: Vec<ScanError>,
}

pub fn scan_directory(source_dir: &PathBuf) -> ScanOutput {
    let mut files = Vec::new();
    let mut errors = Vec::new();

    for entry_result in WalkDir::new(source_dir)
        .follow_links(false)
        .into_iter()
    {
        match entry_result {
            Ok(entry) => {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                if let Some(ext) = path.extension() {
                    let ext_lower = ext.to_string_lossy().to_lowercase();

                    let is_image = IMAGE_EXTENSIONS.contains(&ext_lower.as_str());
                    let is_video = VIDEO_EXTENSIONS.contains(&ext_lower.as_str());

                    if is_image || is_video {
                        files.push(MediaFile {
                            path: path.to_path_buf(),
                            is_video,
                        });
                    }
                }
            }
            Err(e) => {
                errors.push(ScanError {
                    path: e.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    message: e.to_string(),
                });
            }
        }
    }

    ScanOutput { files, errors }
}
