use chrono::NaiveDate;
use md5::{Digest, Md5};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CopyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to create directory: {0}")]
    CreateDir(String),
    #[error("Too many duplicate filenames: {0}")]
    TooManyDuplicates(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum CopyResult {
    Copied { dest: PathBuf },
    Skipped { reason: String },
}

/// Calculate MD5 hash of a file
pub fn calculate_md5(path: &Path) -> Result<String, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Md5::new();
    let mut buffer = [0u8; 65536];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Generate unique filename if file already exists
fn get_unique_filename(dest_dir: &Path, original_name: &str) -> Result<PathBuf, CopyError> {
    let path = dest_dir.join(original_name);
    if !path.exists() {
        return Ok(path);
    }

    let stem = Path::new(original_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let extension = Path::new(original_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    for counter in 1..=10000 {
        let new_name = if extension.is_empty() {
            format!("{}_{}", stem, counter)
        } else {
            format!("{}_{}.{}", stem, counter, extension)
        };
        let new_path = dest_dir.join(&new_name);
        if !new_path.exists() {
            return Ok(new_path);
        }
    }

    Err(CopyError::TooManyDuplicates(original_name.to_string()))
}

/// Copy a single file to destination directory organized by date
pub fn copy_file(
    source: &Path,
    dest_base: &Path,
    date: Option<NaiveDate>,
) -> Result<CopyResult, CopyError> {
    // Use 1970-01-01 for files without metadata
    let date = date.unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());

    // Create date-based directory: YYYY-MM-DD
    let date_dir = dest_base.join(date.format("%Y-%m-%d").to_string());

    if !date_dir.exists() {
        fs::create_dir_all(&date_dir)
            .map_err(|_| CopyError::CreateDir(date_dir.display().to_string()))?;
    }

    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let dest_path = dest_dir_with_dedup(&date_dir, source, file_name)?;

    match dest_path {
        Some(path) => {
            fs::copy(source, &path)?;
            Ok(CopyResult::Copied { dest: path })
        }
        None => Ok(CopyResult::Skipped {
            reason: "Duplicate file (same MD5)".to_string(),
        }),
    }
}

/// Check for duplicates and return destination path or None if should skip
fn dest_dir_with_dedup(
    dest_dir: &Path,
    source: &Path,
    file_name: &str,
) -> Result<Option<PathBuf>, CopyError> {
    let initial_dest = dest_dir.join(file_name);

    if !initial_dest.exists() {
        return Ok(Some(initial_dest));
    }

    // File exists, check MD5
    let source_md5 = calculate_md5(source)?;
    let existing_md5 = calculate_md5(&initial_dest)?;

    if source_md5 == existing_md5 {
        // Same file, skip
        return Ok(None);
    }

    // Different file, generate unique name
    Ok(Some(get_unique_filename(dest_dir, file_name)?))
}
