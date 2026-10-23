//! GDI text (`DrawTextA`, `TextOutA`, `GetTextExtentPoint32A`) rendered with a
//! system TrueType font. The original asks Windows for "Arial" and gets its
//! hinted glyphs; no host reproduces those bytes, so text is the one place the
//! native build deliberately differs from the original's pixels.

use crate::gdi::{self, Object};
use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use s4_x86::Cpu;
use std::sync::OnceLock;

static FONT: OnceLock<Option<FontVec>> = OnceLock::new();

const CANDIDATES: [&str; 6] = [
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
];

fn font() -> Option<&'static FontVec> {
    FONT.get_or_init(|| {
        let paths = std::env::var("S4_FONT").into_iter().chain(CANDIDATES.iter().map(|s| s.to_string()));
        paths.into_iter().find_map(|p| FontVec::try_from_vec(std::fs::read(p).ok()?).ok())
    })
    .as_ref()
}

fn scale_of(font: &FontVec, height: i32) -> PxScale {
    // Positive: cell height (ascent + descent); negative: the em size.
    if height >= 0 {
        PxScale::from(height.max(1) as f32)
    } else {
        let k = font.height_unscaled() / font.units_per_em().unwrap_or(1000.0);
        PxScale::from(-height as f32 * k)
    }
}

fn dc_font(dc: u32) -> (i32, i32) {
    let h = gdi::dc_state(dc).font;
    gdi::GDI.with(|g| match g.borrow().objects.get(&h) {
        Some(Object::Font { height, weight, .. }) => (*height, *weight),
        _ => (-12, 400),
    })
}

fn advance(font: &FontVec, scale: PxScale, s: &str) -> f32 {
    let f = font.as_scaled(scale);
    s.chars().map(|c| f.h_advance(f.glyph_id(c))).sum()
}

/// (width, height) of `s` in the DC's font.
pub fn extent(dc: u32, s: &str) -> (i32, i32) {
    let Some(font) = font() else { return (8 * s.len() as i32, 16) };
    let (h, _) = dc_font(dc);
    let scale = scale_of(font, h);
    let sc = font.as_scaled(scale);
    (advance(font, scale, s).ceil() as i32, sc.height().ceil() as i32)
}

/// Draws `s` with its top-left at (x, y) in the DC's bitmap.
fn draw_run(c: &Cpu, dc: u32, x: f32, y: i32, s: &str, clip: Option<[i32; 4]>) {
    let (Some(font), Some(bmp)) = (font(), gdi::dc_bitmap(dc)) else { return };
    let st = gdi::dc_state(dc);
    let (h, _) = dc_font(dc);
    let scale = scale_of(font, h);
    let sc = font.as_scaled(scale);
    let baseline = y as f32 + sc.ascent();
    let mut pen = x;
    let (tr, tg, tb) = (st.text_color & 0xff, st.text_color >> 8 & 0xff, st.text_color >> 16 & 0xff);
    for ch in s.chars() {
        let g = font.glyph_id(ch).with_scale_and_position(scale, point(pen, baseline));
        pen += sc.h_advance(g.id);
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                if cov <= 0.0 {
                    return;
                }
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if let Some([l, t, r, b]) = clip {
                    if px < l || px >= r || py < t || py >= b {
                        return;
                    }
                }
                let under = gdi::pixel(c, &bmp, px, py);
                if under == 0xffff_ffff {
                    return;
                }
                let mix = |t: u32, u: u32| (t as f32 * cov + u as f32 * (1.0 - cov)).round() as u32;
                let col = mix(tr, under & 0xff) | mix(tg, under >> 8 & 0xff) << 8 | mix(tb, under >> 16 & 0xff) << 16;
                gdi::put_pixel(c, &bmp, px, py, col);
            });
        }
    }
}

/// `TextOutA(dc, x, y, text, count)`.
pub fn text_out(c: &Cpu, dc: u32, x: i32, y: i32, s: &str) {
    let st = gdi::dc_state(dc);
    let (w, _) = extent(dc, s);
    let x0 = match st.align & 6 {
        6 => x - w / 2,
        2 => x - w,
        _ => x,
    };
    draw_run(c, dc, x0 as f32, y, s, None);
}

const DT_CENTER: u32 = 1;
const DT_RIGHT: u32 = 2;
const DT_VCENTER: u32 = 4;
const DT_BOTTOM: u32 = 8;
const DT_WORDBREAK: u32 = 0x10;
const DT_SINGLELINE: u32 = 0x20;
const DT_CALCRECT: u32 = 0x400;

/// `DrawTextA`: returns the height of the text drawn.
pub fn draw_text(c: &Cpu, dc: u32, s: &str, rect: &mut [i32; 4], flags: u32) -> i32 {
    let (_, lh) = extent(dc, "Ag");
    let width = rect[2] - rect[0];
    // Lines: explicit breaks, then word wrap if asked.
    let mut lines: Vec<String> = Vec::new();
    for para in s.split('\n') {
        let para = para.trim_end_matches('\r');
        if flags & DT_WORDBREAK != 0 && flags & DT_SINGLELINE == 0 {
            let mut cur = String::new();
            for word in para.split(' ') {
                let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if !cur.is_empty() && extent(dc, &trial).0 > width {
                    lines.push(std::mem::take(&mut cur));
                    cur = word.to_string();
                } else {
                    cur = trial;
                }
            }
            lines.push(cur);
        } else {
            lines.push(para.to_string());
            if flags & DT_SINGLELINE != 0 {
                break;
            }
        }
    }
    let total = lh * lines.len() as i32;
    if flags & DT_CALCRECT != 0 {
        let w = lines.iter().map(|l| extent(dc, l).0).max().unwrap_or(0);
        rect[2] = rect[0] + w;
        rect[3] = rect[1] + total;
        return total;
    }
    let height = rect[3] - rect[1];
    let top = if flags & DT_VCENTER != 0 { rect[1] + (height - total) / 2 } else if flags & DT_BOTTOM != 0 { rect[3] - total } else { rect[1] };
    for (i, line) in lines.iter().enumerate() {
        let w = extent(dc, line).0;
        let x = if flags & DT_CENTER != 0 { rect[0] + (width - w) / 2 } else if flags & DT_RIGHT != 0 { rect[2] - w } else { rect[0] };
        draw_run(c, dc, x as f32, top + lh * i as i32, line, if flags & 0x100 != 0 { None } else { Some(*rect) });
    }
    total
}
