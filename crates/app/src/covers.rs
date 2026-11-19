use std::io::Cursor;
use std::path::{Path, PathBuf};

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
pub const EARLIER: usize = 7;
const OWN: &str = "playlist-";

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
    let stamp = snellar_core::clock::unix_millis();
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
    if let Some(path) = local(covers, cover) {
        std::fs::remove_file(path).ok();
    }
}

fn local<'a>(covers: &Path, cover: &'a str) -> Option<&'a Path> {
    cover.strip_prefix("file://").map(Path::new).filter(|p| p.parent() == Some(covers))
}

fn url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn owner(name: &str) -> Option<(i64, u64)> {
    let (stem, _) = name.strip_prefix(OWN)?.rsplit_once('.')?;
    let (id, stamp) = stem.split_once('-')?;
    Some((id.parse().ok()?, stamp.parse().ok()?))
}

fn owned(path: &Path) -> Option<(i64, u64)> {
    owner(path.file_name()?.to_str()?)
}

fn own_files(covers: &Path) -> Vec<(i64, u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(covers) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let (id, stamp) = owned(&e.path())?;
            Some((id, stamp, e.path()))
        })
        .collect()
}

fn ranked(covers: &Path, id: i64, current: &str) -> Vec<PathBuf> {
    let current = local(covers, current);
    let mut mine: Vec<_> = own_files(covers).into_iter().filter(|(of, _, path)| *of == id && Some(path.as_path()) != current).collect();
    mine.sort_by_key(|a| std::cmp::Reverse(a.1));
    mine.into_iter().map(|(_, _, path)| path).collect()
}

pub fn earlier(covers: &Path, id: i64, current: &str) -> Vec<String> {
    ranked(covers, id, current).iter().take(EARLIER).map(|p| url(p)).collect()
}

fn prune(covers: &Path, id: i64, current: &str) {
    for path in ranked(covers, id, current).iter().skip(EARLIER) {
        std::fs::remove_file(path).ok();
    }
}

fn free_name(covers: &Path, id: i64, mut stamp: u64, kind: &str) -> PathBuf {
    loop {
        let path = covers.join(format!("{OWN}{id}-{stamp}.{kind}"));
        if !path.exists() {
            return path;
        }
        stamp += 1;
    }
}

pub fn keep(covers: &Path, id: i64, old: &str, current: &str) {
    if let Some(path) = local(covers, old).filter(|p| owned(p).is_none() && local(covers, current) != Some(*p)) {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let stamp = name.rsplit('-').next().and_then(|s| s.parse().ok()).unwrap_or_else(snellar_core::clock::unix_millis);
        let kind = path.extension().and_then(|s| s.to_str()).unwrap_or("jpg");
        std::fs::rename(path, free_name(covers, id, stamp, kind)).ok();
    }
    prune(covers, id, current);
}

pub fn reuse(covers: &Path, id: i64, cover: &str) -> Result<String> {
    let path = local(covers, cover)
        .filter(|p| owned(p).is_some_and(|(of, _)| of == id) && p.is_file())
        .ok_or_else(|| Code::ApiArgs.err(format!("{cover} is not an earlier cover of playlist {id}")))?;
    let kind = path.extension().and_then(|s| s.to_str()).unwrap_or("jpg");
    let to = free_name(covers, id, snellar_core::clock::unix_millis(), kind);
    std::fs::rename(path, &to).context(Code::LibraryFailed)?;
    Ok(url(&to))
}

pub fn sweep(covers: &Path, live: &[(i64, String)]) {
    let files = own_files(covers);
    for (id, _, path) in &files {
        if !live.iter().any(|(of, _)| of == id) {
            std::fs::remove_file(path).ok();
        }
    }
    for (id, current) in live.iter().filter(|(id, _)| files.iter().any(|f| f.0 == *id)) {
        prune(covers, *id, current);
    }
}
