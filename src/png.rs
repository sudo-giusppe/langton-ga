//! Dependency-free PNG encoder (truecolor, 8 bits per channel).
//!
//! Deflate "stored" blocks are used, so the file is not compressed, but any
//! PNG reader accepts it.  This keeps the crate at zero external dependencies
//! while still producing publication-quality figures for the paper.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

fn write_chunk<W: Write>(w: &mut W, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(kind)?;
    w.write_all(data)?;
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    w.write_all(&crc32(&crc_input).to_be_bytes())?;
    Ok(())
}

fn write_zlib_stored<W: Write>(w: &mut W, raw: &[u8]) -> io::Result<()> {
    // zlib header: deflate, 32K window, no preset dictionary, fastest.
    w.write_all(&[0x78, 0x01])?;
    let mut offset = 0;
    while offset < raw.len() {
        let take = (raw.len() - offset).min(65_535);
        let final_block = offset + take >= raw.len();
        w.write_all(&[if final_block { 1 } else { 0 }])?;
        let len = take as u16;
        w.write_all(&len.to_le_bytes())?;
        w.write_all(&(!len).to_le_bytes())?;
        w.write_all(&raw[offset..offset + take])?;
        offset += take;
    }
    if raw.is_empty() {
        w.write_all(&[1, 0, 0, 0xFF, 0xFF])?;
    }
    w.write_all(&adler32(raw).to_be_bytes())?;
    Ok(())
}

/// Write an RGB image.  `pixels` must contain `width * height * 3` bytes in
/// row-major order.
pub fn write_rgb(path: &Path, width: usize, height: usize, pixels: &[u8]) -> io::Result<()> {
    assert_eq!(pixels.len(), width * height * 3);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for y in 0..height {
        raw.push(0); // filter: none
        let start = y * width * 3;
        raw.extend_from_slice(&pixels[start..start + width * 3]);
    }

    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    w.write_all(&SIGNATURE)?;

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut w, b"IHDR", &ihdr)?;

    let mut idat = Vec::new();
    write_zlib_stored(&mut idat, &raw)?;
    write_chunk(&mut w, b"IDAT", &idat)?;
    write_chunk(&mut w, b"IEND", &[])?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn writes_a_small_png() {
        let dir = std::env::temp_dir().join("langton-ga-png-test");
        let path = dir.join("x.png");
        let pixels = vec![255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        write_rgb(&path, 2, 2, &pixels).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], &SIGNATURE);
        assert!(bytes.len() > 50);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
