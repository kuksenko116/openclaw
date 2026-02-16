// Image loading utilities for multimodal support.
//
// Provides helpers to detect image files by extension and load them as
// base64-encoded Image content blocks for the Anthropic API.

use std::path::Path;

use anyhow::{Context, Result};

use super::types::{ContentBlock, ImageSource};

/// Image file extensions we support.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Maximum image file size (20 MB).
const MAX_IMAGE_SIZE: u64 = 20 * 1024 * 1024;

/// Check if a file path has an image extension.
pub(crate) fn is_image_path(path: &str) -> bool {
    let path = Path::new(path);
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Determine the MIME type for an image based on its extension.
fn media_type_for_extension(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;

    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Load an image file from disk and return it as an Image content block.
///
/// The image data is base64-encoded for transmission to the Anthropic API.
pub(crate) async fn load_image_from_path(path: &str) -> Result<ContentBlock> {
    let file_path = Path::new(path);

    // Check file exists
    let metadata = tokio::fs::metadata(file_path)
        .await
        .with_context(|| format!("Cannot access image file: {}", path))?;

    // Check file size
    if metadata.len() > MAX_IMAGE_SIZE {
        anyhow::bail!(
            "Image file too large: {} bytes (max {} bytes)",
            metadata.len(),
            MAX_IMAGE_SIZE
        );
    }

    // Determine media type
    let media_type = media_type_for_extension(path)
        .ok_or_else(|| anyhow::anyhow!("Unsupported image format: {}", path))?;

    // Read and encode
    let data = tokio::fs::read(file_path)
        .await
        .with_context(|| format!("Failed to read image file: {}", path))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);

    Ok(ContentBlock::Image {
        source: ImageSource {
            source_type: "base64".to_string(),
            media_type: media_type.to_string(),
            data: encoded,
        },
    })
}

/// Scan a user input string for references to image files that exist on disk.
///
/// Returns a list of file paths that appear to be image references.
/// Simple heuristic: any whitespace-separated token that looks like an image
/// path (ends with an image extension) and exists on disk.
pub(crate) fn detect_image_paths(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .filter(|token| {
            if !is_image_path(token) {
                return false;
            }
            // Check if the file actually exists (blocking, but quick for path checks)
            Path::new(token).exists()
        })
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_image_path_png() {
        assert!(is_image_path("/tmp/photo.png"));
        assert!(is_image_path("image.PNG"));
    }

    #[test]
    fn test_is_image_path_jpeg() {
        assert!(is_image_path("photo.jpg"));
        assert!(is_image_path("photo.jpeg"));
        assert!(is_image_path("photo.JPEG"));
    }

    #[test]
    fn test_is_image_path_gif_webp() {
        assert!(is_image_path("animation.gif"));
        assert!(is_image_path("photo.webp"));
    }

    #[test]
    fn test_is_image_path_non_image() {
        assert!(!is_image_path("document.pdf"));
        assert!(!is_image_path("code.rs"));
        assert!(!is_image_path("readme.md"));
        assert!(!is_image_path("no_extension"));
    }

    #[test]
    fn test_media_type_for_extension() {
        assert_eq!(media_type_for_extension("photo.png"), Some("image/png"));
        assert_eq!(media_type_for_extension("photo.jpg"), Some("image/jpeg"));
        assert_eq!(media_type_for_extension("photo.jpeg"), Some("image/jpeg"));
        assert_eq!(media_type_for_extension("photo.gif"), Some("image/gif"));
        assert_eq!(media_type_for_extension("photo.webp"), Some("image/webp"));
        assert_eq!(media_type_for_extension("photo.pdf"), None);
    }

    #[test]
    fn test_detect_image_paths_none() {
        let paths = detect_image_paths("hello world no images here");
        assert!(paths.is_empty());
    }

    #[test]
    fn test_detect_image_paths_nonexistent() {
        // These image paths don't exist on disk, so they should not be detected
        let paths = detect_image_paths("look at /tmp/nonexistent_abc123.png please");
        assert!(paths.is_empty());
    }

    #[tokio::test]
    async fn test_load_image_nonexistent() {
        let result = load_image_from_path("/tmp/nonexistent_image_xyz123.png").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_image_unsupported_format() {
        // Create a temp file with unsupported extension
        let tmp = format!("/tmp/openclaw_test_{}.bmp", uuid::Uuid::new_v4());
        tokio::fs::write(&tmp, b"fake bmp data").await.unwrap();

        let result = load_image_from_path(&tmp).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn test_load_image_success() {
        // Create a small fake PNG file
        let tmp = format!("/tmp/openclaw_test_{}.png", uuid::Uuid::new_v4());
        tokio::fs::write(&tmp, b"\x89PNG\r\n\x1a\nfake png data")
            .await
            .unwrap();

        let result = load_image_from_path(&tmp).await.unwrap();
        match result {
            ContentBlock::Image { source } => {
                assert_eq!(source.source_type, "base64");
                assert_eq!(source.media_type, "image/png");
                assert!(!source.data.is_empty());
            }
            other => panic!("Expected Image block, got {:?}", other),
        }

        let _ = tokio::fs::remove_file(&tmp).await;
    }
}
