//! Rendering of visited patterns to PNG (for the paper) and ASCII (for the
//! terminal).

use crate::grid::Grid;
use crate::png;
use crate::sim::SimOutcome;
use std::io;
use std::path::Path;

/// Okabe-Ito derived palette plus a light gray for visited cells of color 0.
const PALETTE: [[u8; 3]; 8] = [
    [0xE8, 0xE8, 0xE2], // 0: visited, neutral
    [0x00, 0x72, 0xB2], // 1: blue
    [0xD5, 0x5E, 0x00], // 2: vermillion
    [0x00, 0x9E, 0x73], // 3: bluish green
    [0xCC, 0x79, 0xA7], // 4: reddish purple
    [0xE6, 0x9F, 0x00], // 5: orange
    [0x56, 0xB4, 0xE9], // 6: sky blue
    [0x00, 0x00, 0x00], // 7: black
];
const BACKGROUND: [u8; 3] = [0xFF, 0xFF, 0xFF];

#[derive(Clone, Copy)]
pub struct RenderOptions {
    /// Maximum number of cells along each axis (larger patterns are cropped).
    pub max_cells: usize,
    /// Pixels per cell.
    pub scale: usize,
    /// If set, center the crop on this cell instead of the bounding-box center.
    pub focus: Option<(i32, i32)>,
    /// Extra cells of background added on every side of the crop.
    pub pad: i32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            max_cells: 600,
            scale: 2,
            focus: None,
            pad: 0,
        }
    }
}

pub struct Rendered {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub cells: usize,
}

/// Render the visited cells of `outcome`, cropping around `focus` or the
/// bounding-box center when the pattern is larger than `max_cells`.
pub fn write_png(outcome: &SimOutcome, path: &Path, opts: &RenderOptions) -> io::Result<Rendered> {
    let grid = &outcome.grid;
    let bbox = grid.bbox();
    let max_cells = opts.max_cells.max(1) as i32;
    let (cx, cy) = match (opts.focus, bbox) {
        (Some(f), _) => f,
        (None, Some(b)) => {
            let (x, y) = b.center();
            (x as i32, y as i32)
        }
        (None, None) => (0, 0),
    };
    let pad = opts.pad.max(0);
    let width_cells = bbox
        .map(|b| b.width().min(max_cells as u64) as i32)
        .unwrap_or(1)
        + 2 * pad;
    let height_cells = bbox
        .map(|b| b.height().min(max_cells as u64) as i32)
        .unwrap_or(1)
        + 2 * pad;
    let x0 = cx - width_cells / 2;
    let y0 = cy - height_cells / 2;
    let scale = opts.scale.max(1);
    let width = width_cells as usize * scale;
    let height = height_cells as usize * scale;
    let mut pixels = vec![0u8; width * height * 3];
    for py in 0..height {
        let cell_y = y0 + (py / scale) as i32;
        for px in 0..width {
            let cell_x = x0 + (px / scale) as i32;
            let color = if grid.is_seen(cell_x, cell_y) {
                PALETTE[grid.read(cell_x, cell_y) as usize % PALETTE.len()]
            } else {
                BACKGROUND
            };
            let idx = (py * width + px) * 3;
            pixels[idx..idx + 3].copy_from_slice(&color);
        }
    }
    png::write_rgb(path, width, height, &pixels)?;
    Ok(Rendered {
        path: path.display().to_string(),
        width,
        height,
        cells: grid.new_cells as usize,
    })
}

/// ASCII view for quick inspection in the terminal.
pub fn ascii(grid: &Grid, max_cells: i32) -> String {
    let Some(b) = grid.bbox() else {
        return "(empty pattern)".to_string();
    };
    let (cx, cy) = b.center();
    let (cx, cy) = (cx as i32, cy as i32);
    let w = (b.width().min(max_cells as u64) as i32).max(1);
    let h = (b.height().min(max_cells as u64) as i32).max(1);
    let x0 = cx - w / 2;
    let y0 = cy - h / 2;
    let glyphs = ['.', '#', '*', '+', 'o', '@', '%', '&'];
    let mut out = String::with_capacity((w as usize + 1) * h as usize);
    for y in 0..h {
        for x in 0..w {
            let (gx, gy) = (x0 + x, y0 + y);
            if !grid.is_seen(gx, gy) {
                out.push(' ');
            } else {
                let c = grid.read(gx, gy) as usize;
                out.push(glyphs[c % glyphs.len()]);
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::Genome;
    use crate::sim::{simulate, SimParams};

    #[test]
    fn renders_classic_pattern() {
        let outcome = simulate(
            &Genome::langton(),
            &SimParams {
                steps: 500,
                ..Default::default()
            },
        );
        let dir = std::env::temp_dir().join("langton-ga-render-test");
        let path = dir.join("pattern.png");
        let r = write_png(
            &outcome,
            &path,
            &RenderOptions {
                scale: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(r.width > 0 && r.height > 0);
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
        let ascii = ascii(&outcome.grid, 40);
        assert!(ascii.contains('#') || ascii.contains('.'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
