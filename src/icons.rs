//! Tray icon rendering: large digits without an outline.
//!
//! Bahnschrift is a variable font, so two-digit and three-digit values are drawn
//! with its bold condensed instance (`wght=700`, `wdth=75`); a single digit uses
//! Segoe UI Bold. The largest font size that still fits the canvas is chosen so
//! the digits fill the icon.

use std::collections::HashMap;
use std::path::PathBuf;

use swash::scale::image::Image;
use swash::scale::{Render, ScaleContext, Source};
use swash::{FontRef, NormalizedCoord};

use crate::protocol::Percent;

/// Size of the rendered canvas in pixels.
pub const SIZE: u32 = crate::ICON_SIZE;
/// Fraction of the canvas used by digits.
const FILL_DIGITS: f32 = 0.98;
/// Fraction of the canvas used by other glyphs.
const FILL_TEXT: f32 = 0.82;
/// Size used to measure the shape of a string before scaling it.
const REFERENCE_PPEM: f32 = 64.0;
/// Largest font size tried, as a multiple of the canvas.
const MAX_PPEM_FACTOR: f32 = 1.3;
/// Smallest font size tried.
const MIN_PPEM: f32 = 6.0;
/// Maximum number of cached icons.
const CACHE_LIMIT: usize = 64;

const SOURCES: [Source; 1] = [Source::Outline];

/// Straight-alpha RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// Raw bytes in RGBA order.
    #[must_use]
    pub const fn as_array(self) -> [u8; 4] {
        self.0
    }
}

/// The four colours used by the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Normal (30-100%, discharging).
    pub normal: Rgba,
    /// Low battery (below 30%).
    pub low: Rgba,
    /// Charging or full.
    pub charge: Rgba,
    /// No fresh data.
    pub stale: Rgba,
}

/// Bright colours for a dark taskbar.
pub const PALETTE_DARK: Palette = Palette {
    normal: Rgba([0, 122, 255, 255]),
    low: Rgba([255, 59, 48, 255]),
    charge: Rgba([0, 140, 70, 255]),
    stale: Rgba([184, 212, 255, 255]),
};

/// Darker colours for a light taskbar, all above 5:1 contrast on `#F3F3F3`.
pub const PALETTE_LIGHT: Palette = Palette {
    normal: Rgba([0, 88, 200, 255]),
    low: Rgba([196, 39, 30, 255]),
    charge: Rgba([0, 107, 52, 255]),
    stale: Rgba([44, 95, 168, 255]),
};

/// Palette for the current taskbar theme.
#[must_use]
pub const fn palette(light: bool) -> Palette {
    if light { PALETTE_LIGHT } else { PALETTE_DARK }
}

/// Colour of the digits: green while charging, red below 30%, otherwise blue.
#[must_use]
pub fn battery_color(percent: Percent, charging: bool, full: bool, light: bool) -> Rgba {
    let palette = palette(light);
    if charging || full {
        palette.charge
    } else if percent < Percent::saturating(30) {
        palette.low
    } else {
        palette.normal
    }
}

/// Multi-digit values use the condensed face so they stay large.
#[must_use]
pub fn uses_condensed(text: &str) -> bool {
    text.len() >= 2 && text.bytes().all(|byte| byte.is_ascii_digit())
}

struct GlyphBitmap {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    data: Vec<u8>,
}

fn variation_coords(font: &FontRef<'_>, condensed: bool) -> Vec<NormalizedCoord> {
    if !condensed {
        return Vec::new();
    }
    // Bahnschrift's bold condensed instance: weight 700, width 75%.
    font.variations()
        .normalized_coords([("wght", 700.0_f32), ("wdth", 75.0_f32)])
        .collect()
}

fn bounds(glyphs: &[GlyphBitmap]) -> (i32, i32, i32, i32) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for glyph in glyphs {
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        min_x = min_x.min(glyph.left);
        min_y = min_y.min(glyph.top);
        max_x = max_x.max(glyph.left + i32::try_from(glyph.width).unwrap_or(0));
        max_y = max_y.max(glyph.top + i32::try_from(glyph.height).unwrap_or(0));
    }
    (min_x, min_y, max_x, max_y)
}

fn render_glyphs(
    data: &[u8],
    coords: &[NormalizedCoord],
    text: &str,
    ppem: f32,
) -> Option<(Vec<GlyphBitmap>, f32)> {
    if ppem < 1.0 {
        return None;
    }
    let font = FontRef::from_index(data, 0)?;
    let charmap = font.charmap();
    let metrics = font.metrics(coords);
    let units_per_em = f32::from(metrics.units_per_em);
    if !(units_per_em.is_finite() && units_per_em > 0.0) {
        return None;
    }
    let glyph_metrics = font.glyph_metrics(coords);
    let mut context = ScaleContext::new();
    let mut scaler = context
        .builder(font)
        .size(ppem)
        .hint(false)
        .normalized_coords(coords.iter().copied())
        .build();
    let renderer = Render::new(&SOURCES);

    let mut pen_x = 0.0_f32;
    let mut glyphs = Vec::new();
    for character in text.chars() {
        let glyph_id = charmap.map(character);
        if glyph_id == 0 {
            return None;
        }
        let advance = glyph_metrics.advance_width(glyph_id) * ppem / units_per_em;
        if let Some(Image {
            placement,
            data: mask,
            ..
        }) = renderer.render(&mut scaler, glyph_id)
        {
            glyphs.push(GlyphBitmap {
                left: pen_x.round() as i32 + placement.left,
                top: placement.top,
                width: placement.width,
                height: placement.height,
                data: mask.to_vec(),
            });
        }
        pen_x += advance;
    }
    if glyphs.is_empty() {
        return None;
    }
    Some((glyphs, pen_x))
}

fn compose(glyphs: &[GlyphBitmap], color: Rgba) -> Vec<u8> {
    let size = SIZE;
    let mut pixels = vec![0_u8; (size as usize) * (size as usize) * 4];
    let (min_x, min_y, max_x, max_y) = bounds(glyphs);
    if min_x > max_x || min_y > max_y {
        return pixels;
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    let offset_x = (i32::try_from(size).unwrap_or(0) - width) / 2 - min_x;
    let offset_y = (i32::try_from(size).unwrap_or(0) - height) / 2 - min_y;

    for glyph in glyphs {
        for y in 0..glyph.height {
            for x in 0..glyph.width {
                let index = (y * glyph.width + x) as usize;
                let Some(&coverage) = glyph.data.get(index) else {
                    continue;
                };
                if coverage == 0 {
                    continue;
                }
                let px = glyph.left + offset_x + i32::try_from(x).unwrap_or(0);
                let py = glyph.top + offset_y + i32::try_from(y).unwrap_or(0);
                if px < 0
                    || py < 0
                    || px >= i32::try_from(size).unwrap_or(0)
                    || py >= i32::try_from(size).unwrap_or(0)
                {
                    continue;
                }
                let pixel = ((py as u32 * size + px as u32) * 4) as usize;
                let alpha = (u16::from(coverage) * u16::from(color.0[3]) / 255) as u8;
                pixels[pixel] = color.0[0];
                pixels[pixel + 1] = color.0[1];
                pixels[pixel + 2] = color.0[2];
                pixels[pixel + 3] = alpha;
            }
        }
    }
    pixels
}

fn fallback(color: Rgba) -> Vec<u8> {
    let mut pixels = vec![0_u8; (SIZE as usize) * (SIZE as usize) * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if x < 8 || y < 8 || x >= SIZE - 8 || y >= SIZE - 8 {
                continue;
            }
            let index = ((y * SIZE + x) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&color.0);
        }
    }
    pixels
}

/// Font stack and icon cache.
#[derive(Debug, Default)]
pub struct IconRenderer {
    bahnschrift: Option<Vec<u8>>,
    segoe: Option<Vec<u8>>,
    arial: Option<Vec<u8>>,
    cache: HashMap<(String, [u8; 4]), Vec<u8>>,
}

fn font_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts")
}

fn read_font(name: &str) -> Option<Vec<u8>> {
    let data = std::fs::read(font_dir().join(name)).ok()?;
    let usable = FontRef::from_index(&data, 0).is_some();
    usable.then_some(data)
}

impl IconRenderer {
    /// Load the system fonts used for the icon.
    #[must_use]
    pub fn new() -> Self {
        let renderer = Self {
            bahnschrift: read_font("bahnschrift.ttf"),
            segoe: read_font("segoeuib.ttf"),
            arial: read_font("arialbd.ttf"),
            cache: HashMap::new(),
        };
        if renderer.bahnschrift.is_none() && renderer.segoe.is_none() && renderer.arial.is_none() {
            log::error!("no usable system font found; tray icon will be a plain marker");
        }
        renderer
    }

    fn candidates(&self, condensed: bool) -> Vec<(&Vec<u8>, bool)> {
        let mut fonts = Vec::new();
        let order: &[(&Option<Vec<u8>>, bool)] = if condensed {
            &[
                (&self.bahnschrift, true),
                (&self.segoe, false),
                (&self.arial, false),
            ]
        } else {
            &[
                (&self.segoe, false),
                (&self.bahnschrift, false),
                (&self.arial, false),
            ]
        };
        for (font, vary) in order {
            if let Some(data) = font {
                fonts.push((data, *vary));
            }
        }
        fonts
    }

    /// Render `text` in `color` and return the RGBA canvas.
    pub fn render(&mut self, text: &str, color: Rgba) -> Vec<u8> {
        let key = (text.to_owned(), color.0);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }

        let condensed = uses_condensed(text);
        let fill = if text.bytes().all(|byte| byte.is_ascii_digit()) {
            FILL_DIGITS
        } else {
            FILL_TEXT
        };
        let max_side = SIZE as f32 * fill;

        let mut pixels = None;
        for (data, vary) in self.candidates(condensed) {
            let font = match FontRef::from_index(data, 0) {
                Some(font) => font,
                None => continue,
            };
            let coords = variation_coords(&font, vary);
            let Some((reference_glyphs, reference_advance)) =
                render_glyphs(data, &coords, text, REFERENCE_PPEM)
            else {
                continue;
            };
            let (min_x, min_y, max_x, max_y) = bounds(&reference_glyphs);
            let reference_width =
                reference_advance.max(f32::from(u16::try_from(max_x - min_x).unwrap_or(1)));
            let reference_height = f32::from(u16::try_from(max_y - min_y).unwrap_or(1));
            if reference_width <= 0.0 || reference_height <= 0.0 {
                continue;
            }
            let by_width = max_side * REFERENCE_PPEM / reference_width;
            let by_height = max_side * REFERENCE_PPEM / reference_height;
            let ppem = by_width
                .min(by_height)
                .min(f32::from(u8::try_from(SIZE).unwrap_or(64)) * MAX_PPEM_FACTOR)
                .max(MIN_PPEM);
            let Some((glyphs, _)) = render_glyphs(data, &coords, text, ppem) else {
                continue;
            };
            pixels = Some(compose(&glyphs, color));
            break;
        }

        let pixels = pixels.unwrap_or_else(|| fallback(color));
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        self.cache.insert(key, pixels.clone());
        pixels
    }
}

/// Bounding box of the opaque part of a canvas, as `(left, top, right, bottom)`.
#[must_use]
pub fn alpha_bbox(rgba: &[u8], size: u32) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = size;
    let mut min_y = size;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for y in 0..size {
        for x in 0..size {
            let index = ((y * size + x) * 4 + 3) as usize;
            if rgba.get(index).copied().unwrap_or(0) > 0 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    found.then_some((min_x, min_y, max_x + 1, max_y + 1))
}
