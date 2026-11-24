//! Lays out, paints and hit-tests an activity's view tree into a 0RGB pixel buffer.
//! No window code here, so all of it runs (and is tested) headless.

use crate::{
    bytes::Result,
    runtime::{android::*, jstr},
    vm::Vm,
};
use fontdue::Font;

/// Android's default TextView size, in sp.
const DEFAULT_TEXT_SP: f32 = 14.0;
const WINDOW_BG: u32 = 0xffff_ffff;
const TEXT_DEFAULT: u32 = 0xff00_0000;
const BUTTON_BG: u32 = 0xffd6_d7d7;
const BUTTON_PAD: [i32; 4] = [16, 12, 16, 12];
const BUTTON_MIN_H: i32 = 48;
const MATCH: i32 = -1;
const WRAP: i32 = -2;

/// A view's box on screen, in pre-order (parents before children, so later = on top).
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub view: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// $DALVINE_FONT, else a common system sans-serif.
pub fn load_font() -> Result<Font> {
    let defaults = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ];
    for path in std::env::var("DALVINE_FONT").into_iter().chain(defaults.map(String::from)) {
        if let Ok(bytes) = std::fs::read(&path) {
            return Font::from_bytes(bytes, Default::default()).map_err(|e| format!("{path}: {e}"));
        }
    }
    Err("no TTF font found; set DALVINE_FONT=/path/to/font.ttf".into())
}

struct Style {
    text: Option<String>,
    size: f32,
    color: u32,
    bg: Option<u32>,
    pad: [i32; 4],
    button: bool,
}

fn style(vm: &Vm, v: u32) -> Result<Style> {
    let f = |key| vm.field(v, key);
    let button = vm.instance_of(v, "Landroid/widget/Button;")?;
    let mut pad = if button { BUTTON_PAD } else { [0; 4] };
    if f(PADDING[0])?.is_some() {
        for (side, key) in PADDING.iter().enumerate() {
            pad[side] = f(key)?.unwrap_or(0) as i32;
        }
    }
    Ok(Style {
        text: f(TEXT)?.map(|t| jstr(vm, t as u32)).transpose()?,
        size: f(TEXT_SIZE)?.map_or(DEFAULT_TEXT_SP, |b| f32::from_bits(b as u32)),
        color: f(TEXT_COLOR)?.map_or(TEXT_DEFAULT, |c| c as u32),
        bg: f(BACKGROUND)?.map(|c| c as u32).or(button.then_some(BUTTON_BG)),
        pad,
        button,
    })
}

/// (width, height) specs from the view's LayoutParams: MATCH, WRAP or px. No params = wrap.
fn spec(vm: &Vm, v: u32) -> Result<(i32, i32)> {
    match vm.field(v, LAYOUT_PARAMS)?.filter(|&lp| lp != 0) {
        None => Ok((WRAP, WRAP)),
        Some(lp) => Ok((
            vm.field(lp as u32, WIDTH)?.map_or(WRAP, |w| w as i32),
            vm.field(lp as u32, HEIGHT)?.map_or(WRAP, |h| h as i32),
        )),
    }
}

fn margins(vm: &Vm, v: u32) -> Result<[i32; 4]> {
    let mut m = [0; 4];
    if let Some(lp) = vm.field(v, LAYOUT_PARAMS)?.filter(|&lp| lp != 0) {
        for (side, slot) in m.iter_mut().enumerate() {
            *slot = margin(vm, lp as u32, side)?;
        }
    }
    Ok(m)
}

fn text_size(font: &Font, s: &str, px: f32) -> (i32, i32) {
    let w: f32 = s.chars().map(|c| font.metrics(c, px).advance_width).sum();
    let h = font.horizontal_line_metrics(px).map_or(px * 1.2, |m| m.new_line_size);
    (w.ceil() as i32, h.ceil() as i32)
}

/// Size the view wants inside `avail` (Android's measure pass, simplified).
/// shortcut: single-line text, LinearLayout-style stacking only, no weight/gravity; extend as apps need.
fn measure(vm: &Vm, font: &Font, v: u32, avail: (i32, i32), depth: usize) -> Result<(i32, i32)> {
    if depth > 64 {
        return Err("view tree deeper than 64".into());
    }
    let st = style(vm, v)?;
    let inner = (avail.0 - st.pad[0] - st.pad[2], avail.1 - st.pad[1] - st.pad[3]);
    let (mut cw, mut ch) = st.text.as_deref().map_or((0, 0), |t| text_size(font, t, st.size));
    let vertical = vm.field(v, ORIENTATION)? == Some(1);
    for k in children(vm, v)? {
        let m = margins(vm, k)?;
        let (kw, kh) = measure(vm, font, k, (inner.0 - m[0] - m[2], inner.1 - m[1] - m[3]), depth + 1)?;
        let (kw, kh) = (kw + m[0] + m[2], kh + m[1] + m[3]);
        if vertical {
            (cw, ch) = (cw.max(kw), ch + kh);
        } else {
            (cw, ch) = (cw + kw, ch.max(kh));
        }
    }
    let (sw, sh) = spec(vm, v)?;
    let pick = |spec: i32, avail: i32, wrap: i32| match spec {
        MATCH => avail,
        WRAP => wrap.min(avail),
        px => px,
    };
    let w = pick(sw, avail.0, cw + st.pad[0] + st.pad[2]);
    let mut h = pick(sh, avail.1, ch + st.pad[1] + st.pad[3]);
    if st.button {
        h = h.max(BUTTON_MIN_H);
    }
    Ok((w.max(0), h.max(0)))
}

fn place(vm: &Vm, font: &Font, v: u32, b: Placed, out: &mut Vec<Placed>, depth: usize) -> Result<()> {
    out.push(b);
    let st = style(vm, v)?;
    let vertical = vm.field(v, ORIENTATION)? == Some(1);
    let (left, top) = (b.x + st.pad[0], b.y + st.pad[1]);
    let (right, bottom) = (b.x + b.w - st.pad[2], b.y + b.h - st.pad[3]);
    let (mut cx, mut cy) = (left, top);
    for k in children(vm, v)? {
        let m = margins(vm, k)?;
        let avail = (right - cx - m[0] - m[2], bottom - cy - m[1] - m[3]); // what's left along the stacking axis
        let avail = if vertical { (right - left - m[0] - m[2], avail.1) } else { (avail.0, bottom - top - m[1] - m[3]) };
        let (w, h) = measure(vm, font, k, avail, depth + 1)?;
        place(vm, font, k, Placed { view: k, x: cx + m[0], y: cy + m[1], w, h }, out, depth + 1)?;
        if vertical {
            cy += h + m[1] + m[3];
        } else {
            cx += w + m[0] + m[2];
        }
    }
    Ok(())
}

/// Boxes for every view of the activity in a `w`×`h` window; the content view fills the window.
pub fn layout(vm: &Vm, font: &Font, activity: u32, w: i32, h: i32) -> Result<Vec<Placed>> {
    let mut out = vec![];
    if let Some(root) = vm.field(activity, CONTENT_VIEW)?.filter(|&r| r != 0) {
        place(vm, font, root as u32, Placed { view: root as u32, x: 0, y: 0, w, h }, &mut out, 0)?;
    }
    Ok(out)
}

/// Draws the laid-out views into a fresh 0RGB buffer (minifb's format).
pub fn paint(vm: &Vm, font: &Font, placed: &[Placed], w: usize, h: usize) -> Result<Vec<u32>> {
    let mut buf = vec![WINDOW_BG & 0xff_ffff; w * h];
    for p in placed {
        let st = style(vm, p.view)?;
        if let Some(bg) = st.bg {
            fill(&mut buf, w, h, p, bg);
        }
        if let Some(text) = &st.text {
            let (tw, th) = text_size(font, text, st.size);
            let (x, y) = if st.button {
                (p.x + (p.w - tw) / 2, p.y + (p.h - th) / 2) // Android buttons center their label
            } else {
                (p.x + st.pad[0], p.y + st.pad[1])
            };
            draw_text(&mut buf, w, h, font, text, st.size, x, y, st.color);
        }
    }
    Ok(buf)
}

/// Delivers a tap at (x, y) to the topmost view with a click listener. Returns whether one was hit.
pub fn tap(vm: &mut Vm, placed: &[Placed], x: i32, y: i32) -> Result<bool> {
    for p in placed.iter().rev() {
        let inside = x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h;
        if let (true, Some(listener)) = (inside, vm.field(p.view, ON_CLICK)?.filter(|&l| l != 0)) {
            vm.invoke_virtual("onClick(Landroid/view/View;)V", &[listener as u32, p.view])?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Binary PPM (P6), the simplest image format any viewer opens.
pub fn ppm(buf: &[u32], w: usize, h: usize) -> Vec<u8> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.extend(buf.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8]));
    out
}

/// Alpha-blends ARGB `color` at `coverage` (0-255) onto a 0RGB pixel.
fn blend(dst: u32, color: u32, coverage: u32) -> u32 {
    let a = (color >> 24) * coverage / 255;
    let ch = |shift: u32| {
        let (d, s) = ((dst >> shift) & 0xff, (color >> shift) & 0xff);
        ((d * (255 - a) + s * a) / 255) << shift
    };
    ch(16) | ch(8) | ch(0)
}

fn fill(buf: &mut [u32], w: usize, h: usize, p: &Placed, color: u32) {
    let (x0, y0) = (p.x.max(0) as usize, p.y.max(0) as usize);
    let (x1, y1) = (((p.x + p.w).max(0) as usize).min(w), ((p.y + p.h).max(0) as usize).min(h));
    for y in y0..y1 {
        for px in &mut buf[y * w + x0..y * w + x1.max(x0)] {
            *px = blend(*px, color, 255);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(buf: &mut [u32], w: usize, h: usize, font: &Font, text: &str, size: f32, x: i32, y: i32, color: u32) {
    let ascent = font.horizontal_line_metrics(size).map_or(size, |m| m.ascent);
    let baseline = y + ascent.round() as i32;
    let mut pen = x as f32;
    // shortcut: glyphs are re-rasterized every frame; cache by (char, size) if repaints get slow
    for c in text.chars() {
        let (m, cov) = font.rasterize(c, size);
        let gx = pen.round() as i32 + m.xmin;
        let gy = baseline - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for col in 0..m.width {
                let (px, py) = (gx + col as i32, gy + row as i32);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let i = py as usize * w + px as usize;
                    buf[i] = blend(buf[i], color, cov[row * m.width + col] as u32);
                }
            }
        }
        pen += m.advance_width;
    }
}

#[cfg(test)]
mod tests {
    use super::blend;

    #[test]
    fn blending() {
        assert_eq!(blend(0x000000, 0xffffffff, 255), 0xffffff); // opaque white over black
        assert_eq!(blend(0x123456, 0xffffffff, 0), 0x123456); // zero coverage leaves the pixel
        assert_eq!(blend(0x000000, 0x80ff0000, 255), 0x800000); // 50% red
    }
}
