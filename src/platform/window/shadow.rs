use crate::theme::ShadowLayer;
use anyhow::{Context, Result};
use gpui::Rgba;
use std::f32::consts::SQRT_2;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindowVisible, RegisterClassW, SET_WINDOW_POS_FLAGS, SW_HIDE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
    WINDOW_EX_STYLE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

#[cfg(test)]
mod tests;

const CLASS: PCWSTR = w!("Vestigo.shadow");
const REACH: f32 = 3.0;
const CANVAS_STEP: usize = 256;
const ERF_SCALE: f32 = 0.327_591_1;
const ERF_TERMS: [f32; 5] = [0.254_829_6, -0.284_496_74, 1.421_413_7, -1.453_152, 1.061_405_4];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub(super) layers: Vec<ShadowLayer>,
    pub(super) radii: [f32; 4],
}

pub(super) struct Shadow {
    hwnd: HWND,
    wanted: bool,
    painted: Option<(SIZE, i32, Look)>,
    canvas: Option<Canvas>,
    alpha: u8,
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: usize,
    height: usize,
}

impl Shadow {
    pub(super) fn create(extra: WINDOW_EX_STYLE) -> Result<Self> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.context("reading the module handle")?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(shadow_proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
        let style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | extra;
        let hwnd =
            unsafe { CreateWindowExW(style, CLASS, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None) }
                .context("creating the shadow window")?;
        Ok(Self { hwnd, wanted: false, painted: None, canvas: None, alpha: u8::MAX })
    }

    pub(super) fn place(&mut self, owner: HWND, frame: RECT, scale: f32, look: &Look) {
        self.wanted = !look.layers.is_empty();
        if !self.wanted {
            self.hide();
            return;
        }
        let margin = margin(&look.layers, scale);
        let origin = POINT { x: frame.left - margin, y: frame.top - margin };
        let size = SIZE { cx: frame.right - frame.left + 2 * margin, cy: frame.bottom - frame.top + 2 * margin };
        let mut flags = SWP_NOSIZE | SWP_NOACTIVATE;
        if self.painted.as_ref().map(|(size, margin, look)| (*size, *margin, look)) != Some((size, margin, look)) {
            let painted = self.paint(origin, size, margin, scale, look);
            self.painted = painted.then(|| (size, margin, look.clone()));
            flags |= SWP_NOMOVE;
        }
        if unsafe { IsWindowVisible(owner) }.as_bool() {
            flags |= SWP_SHOWWINDOW;
        }
        below(self.hwnd, owner, origin, flags);
    }

    pub(super) fn follow(&self, owner: HWND, frame: RECT) {
        if let (true, Some((_, margin, _))) = (self.wanted, &self.painted) {
            let origin = POINT { x: frame.left - margin, y: frame.top - margin };
            below(self.hwnd, owner, origin, SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub(super) fn show(&self, owner: HWND) {
        if self.wanted {
            below(self.hwnd, owner, POINT::default(), SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    pub(super) fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub(super) fn fade(&mut self, share: f32) {
        let alpha = (share.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;
        if std::mem::replace(&mut self.alpha, alpha) == alpha || self.painted.is_none() {
            return;
        }
        let blend = self.blend();
        unsafe {
            let _ = UpdateLayeredWindow(self.hwnd, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    }

    fn blend(&self) -> BLENDFUNCTION {
        BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: self.alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        }
    }

    fn paint(&mut self, origin: POINT, size: SIZE, margin: i32, scale: f32, look: &Look) -> bool {
        let (Ok(width), Ok(height)) = (usize::try_from(size.cx), usize::try_from(size.cy)) else { return false };
        let hwnd = self.hwnd;
        let blend = self.blend();
        let Some(canvas) = self.canvas(width, height) else { return false };
        let stride = canvas.width;
        draw(canvas.pixels(), stride, width, height, margin as f32, scale, look);
        let source = POINT::default();
        let painted = unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&origin),
                Some(&size),
                Some(canvas.dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        };
        if let Err(error) = &painted {
            log::warn!("Couldn't paint the window shadow: {error}");
        }
        painted.is_ok()
    }

    fn canvas(&mut self, width: usize, height: usize) -> Option<&mut Canvas> {
        let held = self.canvas.as_ref().map_or((0, 0), |canvas| (canvas.width, canvas.height));
        if held.0 < width || held.1 < height {
            self.canvas = None;
            let grown = |held: usize, wanted: usize| held.max(wanted.next_multiple_of(CANVAS_STEP));
            self.canvas = Canvas::new(grown(held.0, width), grown(held.1, height));
        }
        self.canvas.as_mut()
    }
}

impl Drop for Shadow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn shadow_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn below(hwnd: HWND, owner: HWND, origin: POINT, flags: SET_WINDOW_POS_FLAGS) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(owner), origin.x, origin.y, 0, 0, flags);
    }
}

fn sigma(layer: &ShadowLayer, scale: f32) -> f32 {
    layer.blur / 2.0 * scale
}

fn margin(layers: &[ShadowLayer], scale: f32) -> i32 {
    let reach = |layer: &ShadowLayer| {
        layer.x.abs().max(layer.y.abs()) * scale + layer.spread.max(0.0) * scale + REACH * sigma(layer, scale)
    };
    layers.iter().map(reach).fold(0.0, f32::max).ceil() as i32
}

impl Canvas {
    fn new(width: usize, height: usize) -> Option<Self> {
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: i32::try_from(width).ok()?,
            biHeight: -i32::try_from(height).ok()?,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let previous = SelectObject(dc, bitmap.into());
            Some(Self { dc, bitmap, previous, bits: bits.cast(), width, height })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.width * self.height) }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

fn draw(target: &mut [u32], stride: usize, width: usize, height: usize, margin: f32, scale: f32, look: &Look) {
    if width > stride || target.len() < stride * height {
        return;
    }
    let panel = Panel::new(margin, width as f32 - margin, margin, height as f32 - margin, look.radii);
    let casts: Vec<(Panel, f32)> = look
        .layers
        .iter()
        .map(|layer| {
            let cast = panel.cast(layer.x * scale, layer.y * scale, layer.spread * scale);
            (cast, (sigma(layer, scale) * SQRT_2).max(f32::EPSILON))
        })
        .collect();
    let plain = |near: fn(&Panel) -> f32, far: fn(&Panel) -> f32| {
        let curve = |panel: &Panel| panel.radii.into_iter().fold(0.0, f32::max);
        let (mut lower, mut upper) = (near(&panel) + curve(&panel), far(&panel) - curve(&panel));
        for (cast, spread) in &casts {
            lower = lower.max(near(cast) + REACH * spread + curve(cast));
            upper = upper.min(far(cast) - REACH * spread - curve(cast));
        }
        (lower, upper)
    };
    let (columns, xs) = axis(width, plain(|panel| panel.left, |panel| panel.right));
    let (rows, ys) = axis(height, plain(|panel| panel.top, |panel| panel.bottom));
    let mut shade = vec![[0.0f32; 4]; xs.len() * ys.len()];
    for (layer, (cast, spread)) in look.layers.iter().zip(&casts).rev() {
        let Rgba { r, g, b, a } = Rgba::from(layer.color);
        let strength = a * layer.opacity;
        for (pixel, cover) in shade.iter_mut().zip(blurred(cast, &xs, &ys, *spread)) {
            let alpha = strength * cover;
            for (channel, value) in pixel.iter_mut().zip([r, g, b, 1.0]) {
                *channel = value * alpha + *channel * (1.0 - alpha);
            }
        }
    }
    let line = xs.len().max(1);
    let mut grid = Vec::with_capacity(shade.len());
    for (y, shades) in ys.iter().zip(shade.chunks(line)) {
        for (x, [r, g, b, a]) in xs.iter().zip(shades) {
            let clear = 1.0 - panel.coverage(*x, *y);
            let channel = |value: f32, shift: u32| ((value * clear * f32::from(u8::MAX) + 0.5) as u32) << shift;
            grid.push(channel(*a, 24) | channel(*r, 16) | channel(*g, 8) | channel(*b, 0));
        }
    }
    let mut previous: Option<(usize, &[u32])> = None;
    for (out, row) in target.chunks_mut(stride).zip(rows) {
        let Some(out) = out.get_mut(..width) else { return };
        match previous {
            Some((last, above)) if last == row => out.copy_from_slice(above),
            Some(_) | None => {
                let cells = grid.get(row * line..).unwrap_or_default();
                for (pixel, column) in out.iter_mut().zip(&columns) {
                    *pixel = cells.get(*column).copied().unwrap_or(0);
                }
            }
        }
        previous = Some((row, out));
    }
}

fn axis(length: usize, (lower, upper): (f32, f32)) -> (Vec<usize>, Vec<f32>) {
    let mut map = Vec::with_capacity(length);
    let mut centers: Vec<f32> = Vec::new();
    for index in 0..length {
        let center = index as f32 + 0.5;
        let inside = |center: f32| center > lower && center < upper;
        if !(inside(center) && centers.last().is_some_and(|last| inside(*last))) {
            centers.push(center);
        }
        map.push(centers.len().saturating_sub(1));
    }
    (map, centers)
}

#[derive(Clone, Copy)]
struct Panel {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    radii: [f32; 4],
}

impl Panel {
    fn new(left: f32, right: f32, top: f32, bottom: f32, radii: [f32; 4]) -> Self {
        let most = ((right - left).min(bottom - top) / 2.0).max(0.0);
        Self { left, right, top, bottom, radii: radii.map(|radius| radius.clamp(0.0, most)) }
    }

    fn cast(self, x: f32, y: f32, spread: f32) -> Self {
        let half_width = ((self.right - self.left) / 2.0 + spread).max(0.0);
        let half_height = ((self.bottom - self.top) / 2.0 + spread).max(0.0);
        let (center_x, center_y) = ((self.left + self.right) / 2.0 + x, (self.top + self.bottom) / 2.0 + y);
        let grow = |radius: f32| if radius > 0.0 { radius + spread } else { 0.0 };
        Self::new(
            center_x - half_width,
            center_x + half_width,
            center_y - half_height,
            center_y + half_height,
            self.radii.map(grow),
        )
    }

    fn coverage(&self, x: f32, y: f32) -> f32 {
        if x < self.left || x > self.right || y < self.top || y > self.bottom {
            return 0.0;
        }
        let [top_left, top_right, bottom_right, bottom_left] = self.radii;
        let corners = [
            (self.left + top_left, self.top + top_left, top_left, -1.0, -1.0),
            (self.right - top_right, self.top + top_right, top_right, 1.0, -1.0),
            (self.right - bottom_right, self.bottom - bottom_right, bottom_right, 1.0, 1.0),
            (self.left + bottom_left, self.bottom - bottom_left, bottom_left, -1.0, 1.0),
        ];
        corners
            .into_iter()
            .find(|(center_x, center_y, _, across, down)| (x - center_x) * across > 0.0 && (y - center_y) * down > 0.0)
            .map_or(1.0, |(center_x, center_y, radius, _, _)| {
                (radius - (x - center_x).hypot(y - center_y) + 0.5).clamp(0.0, 1.0)
            })
    }

    fn insets(&self, y: f32) -> (f32, f32) {
        let [top_left, top_right, bottom_right, bottom_left] = self.radii;
        let inset = |radius: f32, from_edge: f32| {
            if from_edge >= radius {
                return 0.0;
            }
            let rise = radius - from_edge;
            radius - (radius * radius - rise * rise).max(0.0).sqrt()
        };
        let (from_top, from_bottom) = (y - self.top, self.bottom - y);
        (
            inset(top_left, from_top).max(inset(bottom_left, from_bottom)),
            inset(top_right, from_top).max(inset(bottom_right, from_bottom)),
        )
    }
}

fn blurred(panel: &Panel, xs: &[f32], ys: &[f32], spread: f32) -> Vec<f32> {
    let across: Vec<f32> = xs.iter().map(|x| band(*x, panel.left, panel.right, spread)).collect();
    let down: Vec<f32> = ys.iter().map(|y| band(*y, panel.top, panel.bottom, spread)).collect();
    let mut cover: Vec<f32> = down.iter().flat_map(|down| across.iter().map(move |across| across * down)).collect();
    let [top_left, top_right, bottom_right, bottom_left] = panel.radii;
    let (above, below) = (top_left.max(top_right).ceil(), bottom_left.max(bottom_right).ceil());
    let slices = (0..above as usize)
        .map(|row| panel.top + row as f32)
        .chain((0..below as usize).map(|row| panel.bottom - below + row as f32));
    let reach = REACH * spread;
    let near = |center: f32, from: f32, to: f32| center > from - reach && center < to + reach;
    for slice in slices {
        let (left, right) = panel.insets(slice + 0.5);
        if left == 0.0 && right == 0.0 {
            continue;
        }
        let cuts = [(panel.left, panel.left + left), (panel.right - right, panel.right)];
        let shares: Vec<(usize, Vec<f32>)> = cuts
            .into_iter()
            .filter(|(start, end)| end > start)
            .map(|(start, end)| {
                let first = xs.partition_point(|x| *x <= start - reach);
                let values = xs.iter().skip(first).take_while(|x| near(**x, start, end));
                (first, values.map(|x| band(*x, start, end, spread)).collect())
            })
            .collect();
        for (y, row) in ys.iter().zip(cover.chunks_mut(xs.len().max(1))) {
            if !near(*y, slice, slice + 1.0) {
                continue;
            }
            let weight = band(*y, slice, slice + 1.0, spread);
            for (first, values) in &shares {
                for (cover, share) in row.iter_mut().skip(*first).zip(values) {
                    *cover = (*cover - share * weight).max(0.0);
                }
            }
        }
    }
    cover
}

fn band(center: f32, start: f32, end: f32, spread: f32) -> f32 {
    0.5 * (erf((center - start) / spread) - erf((center - end) / spread))
}

fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + ERF_SCALE * x.abs());
    let series = ERF_TERMS.iter().rev().fold(0.0, |sum, term| sum * t + term) * t;
    (1.0 - series * (-x * x).exp()).copysign(x)
}
