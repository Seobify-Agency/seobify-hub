use serde::{Deserialize, Serialize};
use std::path::Path;

/// Container formats accepted as input and produced as output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::WebP => "webp",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::WebP => "image/webp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::WebP => "webp",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "jpg" | "jpeg" | "jpe" | "jfif" => Some(ImageFormat::Jpeg),
            "webp" => Some(ImageFormat::WebP),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    #[serde(rename = "hasAlpha")]
    pub has_alpha: bool,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub format: ImageFormat,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

pub const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Detect the container format from magic bytes (never from the file extension).
pub fn detect_format(data: &[u8]) -> Result<ImageFormat, String> {
    if data.len() >= 8 && data[..8] == PNG_MAGIC {
        return Ok(ImageFormat::Png);
    }
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok(ImageFormat::Jpeg);
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(ImageFormat::WebP);
    }
    Err("Unsupported image format: expected PNG, JPEG or WebP".to_string())
}

/// Validate that the file is a PNG by checking magic bytes
pub fn validate_png(data: &[u8]) -> Result<(), String> {
    if data.len() < 8 {
        return Err("File too small to be a valid PNG".to_string());
    }
    if data[..8] != PNG_MAGIC {
        return Err("File is not a valid PNG image".to_string());
    }
    Ok(())
}

/// Width, height and alpha presence read straight from the container headers.
pub fn read_dimensions(data: &[u8]) -> Result<(ImageFormat, u32, u32, bool), String> {
    let format = detect_format(data)?;
    let (w, h, alpha) = match format {
        ImageFormat::Png => png_header(data)?,
        ImageFormat::Jpeg => jpeg_header(data)?,
        ImageFormat::WebP => webp_header(data)?,
    };
    Ok((format, w, h, alpha))
}

fn png_header(data: &[u8]) -> Result<(u32, u32, bool), String> {
    if data.len() < 33 {
        return Err("PNG file too small to contain IHDR chunk".to_string());
    }
    if &data[12..16] != b"IHDR" {
        return Err("Missing IHDR chunk in PNG".to_string());
    }
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    let color_type = data[25];
    // Color type 4 = greyscale+alpha, 6 = RGBA. A tRNS chunk also yields transparency.
    let mut has_alpha = color_type == 4 || color_type == 6;
    if !has_alpha {
        let mut off = 8usize;
        while off + 8 <= data.len() {
            let len = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            let ty = &data[off + 4..off + 8];
            if ty == b"tRNS" {
                has_alpha = true;
                break;
            }
            if ty == b"IDAT" || ty == b"IEND" {
                break;
            }
            off = off.saturating_add(12).saturating_add(len);
        }
    }
    Ok((width, height, has_alpha))
}

fn jpeg_header(data: &[u8]) -> Result<(u32, u32, bool), String> {
    let mut pos = 2usize;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let marker = data[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // Standalone markers carry no length field.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let is_sof = matches!(
            marker,
            0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
        );
        if is_sof {
            if pos + 9 > data.len() || len < 7 {
                return Err("Truncated JPEG SOF segment".to_string());
            }
            let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]) as u32;
            let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]) as u32;
            return Ok((width, height, false));
        }
        pos += 2 + len.max(2);
    }
    Err("JPEG frame header (SOF) not found".to_string())
}

fn webp_header(data: &[u8]) -> Result<(u32, u32, bool), String> {
    if data.len() < 30 {
        return Err("WebP file too small".to_string());
    }
    let riff_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end = (8 + riff_size).min(data.len());
    let mut pos = 12usize;
    let mut canvas: Option<(u32, u32, bool)> = None;
    let mut alpha_chunk = false;
    while pos + 8 <= end {
        let fourcc = &data[pos..pos + 4];
        let size =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        let payload_end = pos + 8 + size;
        if payload_end > data.len() {
            break;
        }
        let payload = &data[pos + 8..payload_end];
        match fourcc {
            b"VP8X" => {
                if payload.len() >= 10 {
                    let flags = payload[0];
                    let w = 1 + (payload[4] as u32 | (payload[5] as u32) << 8 | (payload[6] as u32) << 16);
                    let h = 1 + (payload[7] as u32 | (payload[8] as u32) << 8 | (payload[9] as u32) << 16);
                    canvas = Some((w, h, flags & 0x10 != 0));
                }
            }
            b"VP8 " => {
                if payload.len() >= 10 && payload[3] == 0x9D && payload[4] == 0x01 && payload[5] == 0x2A {
                    let w = (u16::from_le_bytes([payload[6], payload[7]]) & 0x3FFF) as u32;
                    let h = (u16::from_le_bytes([payload[8], payload[9]]) & 0x3FFF) as u32;
                    if canvas.is_none() {
                        canvas = Some((w, h, false));
                    }
                }
            }
            b"VP8L" => {
                if payload.len() >= 5 && payload[0] == 0x2F {
                    let bits = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                    let w = (bits & 0x3FFF) + 1;
                    let h = ((bits >> 14) & 0x3FFF) + 1;
                    let a = (bits >> 28) & 1 == 1;
                    match canvas {
                        None => canvas = Some((w, h, a)),
                        Some((cw, ch, ca)) => canvas = Some((cw, ch, ca || a)),
                    }
                }
            }
            b"ALPH" => alpha_chunk = true,
            _ => {}
        }
        pos = payload_end + (size & 1);
    }
    match canvas {
        Some((w, h, a)) => Ok((w, h, a || alpha_chunk)),
        None => Err("WebP bitstream header (VP8/VP8L/VP8X) not found".to_string()),
    }
}

/// Extract image dimensions, alpha presence and format from the container header
/// of a PNG, JPEG or WebP file.
pub fn extract_image_metadata(data: &[u8], file_path: &str) -> Result<ImageMetadata, String> {
    let (format, width, height, has_alpha) = read_dimensions(data)?;

    let path = Path::new(file_path);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(ImageMetadata {
        width,
        height,
        file_size: data.len() as u64,
        has_alpha,
        file_name,
        file_path: file_path.to_string(),
        format,
        mime_type: format.mime_type().to_string(),
    })
}

/// Extract image dimensions and alpha channel info from PNG IHDR chunk
pub fn extract_png_metadata(data: &[u8], file_path: &str) -> Result<ImageMetadata, String> {
    validate_png(data)?;
    extract_image_metadata(data, file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_png_jpeg_webp_by_magic() {
        let mut png = PNG_MAGIC.to_vec();
        png.extend_from_slice(&[0; 32]);
        assert_eq!(detect_format(&png).unwrap(), ImageFormat::Png);
        assert_eq!(detect_format(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap(), ImageFormat::Jpeg);
        let mut webp = b"RIFF\x04\x00\x00\x00WEBP".to_vec();
        webp.extend_from_slice(&[0; 20]);
        assert_eq!(detect_format(&webp).unwrap(), ImageFormat::WebP);
        assert!(detect_format(b"GIF89a").is_err());
    }

    #[test]
    fn reads_dimensions_from_encoded_images() {
        let img = image::RgbaImage::from_fn(37, 21, |x, y| {
            image::Rgba([x as u8, y as u8, 7, if x > 30 { 128 } else { 255 }])
        });
        let dyn_img = image::DynamicImage::ImageRgba8(img.clone());

        let mut png = std::io::Cursor::new(Vec::new());
        dyn_img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (f, w, h, a) = read_dimensions(png.get_ref()).unwrap();
        assert_eq!((f, w, h, a), (ImageFormat::Png, 37, 21, true));

        let mut jpg = std::io::Cursor::new(Vec::new());
        dyn_img
            .to_rgb8()
            .write_to(&mut jpg, image::ImageFormat::Jpeg)
            .unwrap();
        let (f, w, h, a) = read_dimensions(jpg.get_ref()).unwrap();
        assert_eq!((f, w, h, a), (ImageFormat::Jpeg, 37, 21, false));

        let mut webp = std::io::Cursor::new(Vec::new());
        dyn_img.write_to(&mut webp, image::ImageFormat::WebP).unwrap();
        let (f, w, h, a) = read_dimensions(webp.get_ref()).unwrap();
        assert_eq!((f, w, h, a), (ImageFormat::WebP, 37, 21, true));
    }
}
