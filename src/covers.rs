use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use snellar_core::cache::write_atomic;
use snellar_core::error::Code;

pub const LIMIT: u64 = 20 << 20;
const MAX_SIDE: u32 = 1024;
const MAX_INPUT_SIDE: u32 = 16384;
const MAX_MEMORY: u64 = 512 << 20;
const JPEG_QUALITY: u8 = 88;

fn decode(bytes: &[u8]) -> image::ImageResult<DynamicImage> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_SIDE);
    limits.max_image_height = Some(MAX_INPUT_SIDE);
    limits.max_alloc = Some(MAX_MEMORY);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let turn = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(turn);
    Ok(image)
}

pub(crate) fn shrink(bytes: &[u8]) -> image::ImageResult<(Vec<u8>, &'static str)> {
    let image = decode(bytes)?;
    let image = if image.width().max(image.height()) > MAX_SIDE { image.thumbnail(MAX_SIDE, MAX_SIDE) } else { image };
    let mut out = Vec::new();
    if image.color().has_alpha() {
        image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
        return Ok((out, "png"));
    }
    image.to_rgb8().write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))?;
    Ok((out, "jpg"))
}

pub async fn store(covers: &Path, label: &str, bytes: Vec<u8>) -> Result<String> {
    let (image, kind) = tokio::task::spawn_blocking(move || shrink(&bytes))
        .await
        .context(Code::Internal)?
        .map_err(|e| Code::CoverUnreadable.err(format!("not a usable PNG, JPEG or WebP image: {e}")))?;
    tokio::fs::create_dir_all(covers).await.context(Code::LibraryFailed)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let to = covers.join(format!("{label}-{stamp}.{kind}"));
    write_atomic(&to, image).await.context(Code::LibraryFailed)?;
    Ok(format!("file://{}", to.display()))
}

pub async fn from_file(covers: &Path, id: i64, from: &Path) -> Result<String> {
    let unreadable = |why: String| Code::CoverUnreadable.err(format!("{}: {why}", from.display()));
    let size = tokio::fs::metadata(from).await.map_err(|e| unreadable(e.to_string()))?.len();
    if size > LIMIT {
        return Err(unreadable(format!("{size} bytes, more than {LIMIT}")));
    }
    let bytes = tokio::fs::read(from).await.map_err(|e| unreadable(e.to_string()))?;
    store(covers, &format!("playlist-{id}"), bytes).await
}

pub fn drop_local(covers: &Path, cover: &str) {
    if let Some(path) = cover.strip_prefix("file://").map(Path::new).filter(|p| p.parent() == Some(covers)) {
        std::fs::remove_file(path).ok();
    }
}

#[cfg(test)]
mod tests {
    use image::{GenericImageView, RgbImage, RgbaImage};

    use super::*;

    fn png(image: DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn a_big_image_is_resized_and_a_small_one_keeps_its_size() {
        let (big, kind) = shrink(&png(RgbImage::new(3000, 1500).into())).unwrap();
        assert_eq!(kind, "jpg");
        assert_eq!(image::load_from_memory(&big).unwrap().dimensions(), (1024, 512));
        let (small, _) = shrink(&png(RgbImage::new(300, 200).into())).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().dimensions(), (300, 200));
    }

    #[test]
    fn transparency_survives() {
        let (out, kind) = shrink(&png(RgbaImage::new(64, 64).into())).unwrap();
        assert_eq!(kind, "png");
        assert!(image::load_from_memory(&out).unwrap().color().has_alpha());
    }

    #[test]
    fn what_is_not_a_usable_image_is_refused() {
        for bad in [&b"not an image"[..], b"\x89PNG\r\n\x1a\nrest", b"GIF89a\x01\x00\x01\x00", b""] {
            assert!(shrink(bad).is_err(), "{bad:?}");
        }
        assert!(shrink(&png(RgbImage::new(MAX_INPUT_SIDE + 1, 1).into())).is_err(), "too many pixels to open");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_too_large_or_missing_is_refused_before_it_is_read() {
        let dir = std::env::temp_dir().join(format!("snellar-covers-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let huge = dir.join("huge.png");
        std::fs::File::create(&huge).unwrap().set_len(LIMIT + 1).unwrap();
        let covers = dir.join("covers");
        for path in [huge, dir.join("gone.png")] {
            let e = from_file(&covers, 1, &path).await.unwrap_err();
            assert_eq!(Code::of(&e), Code::CoverUnreadable);
        }
        assert!(!covers.exists(), "nothing was written");
        std::fs::remove_dir_all(&dir).ok();
    }
}
