use chrono::NaiveDate;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::ffprobe_command;

#[derive(Deserialize)]
struct FfprobeOutput {
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    tags: Option<FfprobeTags>,
}

#[derive(Deserialize)]
struct FfprobeTags {
    creation_time: Option<String>,
}

/// Extract creation date from media file
/// Returns tuple of (date, source) where source indicates where the date came from
/// Returns None if no valid date found
pub fn extract_creation_date(path: &Path, is_video: bool) -> Option<(NaiveDate, &'static str)> {
    // Try media metadata first
    let media_date = if is_video {
        extract_video_date(path)
    } else {
        extract_image_date(path)
    };

    if let Some(date) = media_date {
        let source = if is_video { "ffprobe creation_time" } else { "EXIF DateTimeOriginal" };
        return Some((date, source));
    }

    // Fallback to file modification time
    if let Some(date) = extract_file_modified_date(path) {
        return Some((date, "文件修改时间"));
    }

    None
}

/// Extract date from image EXIF data
fn extract_image_date(path: &Path) -> Option<NaiveDate> {
    let file = File::open(path).ok()?;
    let mut bufreader = BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut bufreader).ok()?;

    // Try DateTimeOriginal first, then CreateDate, then DateTime
    let date_fields = [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTimeDigitized,
        exif::Tag::DateTime,
    ];

    for tag in date_fields {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
            if let Some(date) = parse_exif_datetime(&field.display_value().to_string()) {
                return Some(date);
            }
        }
    }

    None
}

/// Extract date from file modification time
fn extract_file_modified_date(path: &Path) -> Option<NaiveDate> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let datetime: chrono::DateTime<chrono::Utc> = modified.into();
    Some(datetime.date_naive())
}

/// Parse EXIF datetime string (format: "YYYY:MM:DD HH:MM:SS")
fn parse_exif_datetime(datetime_str: &str) -> Option<NaiveDate> {
    // EXIF format: "2024:01:15 10:30:45" or "2024-01-15 10:30:45"
    let parts: Vec<&str> = datetime_str.split(|c| c == ' ' || c == 'T').collect();
    if parts.is_empty() {
        return None;
    }

    let date_part = parts[0];
    let date_components: Vec<&str> = date_part.split(|c| c == ':' || c == '-').collect();

    if date_components.len() >= 3 {
        let year: i32 = date_components[0].parse().ok()?;
        let month: u32 = date_components[1].parse().ok()?;
        let day: u32 = date_components[2].parse().ok()?;

        // Validate date components
        if year > 1970 && month >= 1 && month <= 12 && day >= 1 && day <= 31 {
            return NaiveDate::from_ymd_opt(year, month, day);
        }
    }

    None
}

/// Extract date from video metadata using ffprobe
fn extract_video_date(path: &Path) -> Option<NaiveDate> {
    let output = ffprobe_command()
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_entries", "format_tags=creation_time",
            path.to_str()?,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;

    // Parse JSON using serde_json
    let ffprobe_output: FfprobeOutput = serde_json::from_str(&json_str).ok()?;
    let datetime_str = ffprobe_output
        .format?
        .tags?
        .creation_time?;

    parse_iso_datetime(&datetime_str)
}

/// Parse ISO datetime string (format: "2024-01-15T10:30:45.000000Z")
fn parse_iso_datetime(datetime_str: &str) -> Option<NaiveDate> {
    let date_part = datetime_str.split('T').next()?;
    let components: Vec<&str> = date_part.split('-').collect();

    if components.len() >= 3 {
        let year: i32 = components[0].parse().ok()?;
        let month: u32 = components[1].parse().ok()?;
        let day: u32 = components[2].parse().ok()?;

        if year > 1970 && month >= 1 && month <= 12 && day >= 1 && day <= 31 {
            return NaiveDate::from_ymd_opt(year, month, day);
        }
    }

    None
}
