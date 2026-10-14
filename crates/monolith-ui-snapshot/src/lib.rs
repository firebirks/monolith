//! A software renderer of egui frames, for screenshots and visual tests of
//! the desktop client (`docs/adr/0012-design-system.md`).
//!
//! It runs an egui context for a few frames with fixed input, tessellates
//! the last frame, and rasterizes the triangles on the CPU the way egui's
//! OpenGL renderer does: premultiplied colours, blended in gamma space,
//! textures sampled bilinearly. The result depends on no GPU, driver or
//! window system, so the same code gives the same image anywhere, up to
//! the last bit of a few `sin` and `cos` of the tessellator; compare with
//! [`Image::differs`], which allows for that.
//!
//! Test tooling: it handles no peer input, and the arithmetic below works
//! on pixel indices bounded by the image size.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::io;
use std::path::Path;

use egui::epaint::{ClippedPrimitive, Primitive, Vertex};
use egui::{
    Color32, Context, Event, ImageData, Pos2, RawInput, Rect, TextureFilter, TextureId, Vec2,
    ViewportId,
};

/// An RGBA image, 8 bits per channel, not premultiplied, opaque.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Rows from the top, four bytes per pixel.
    pub rgba: Vec<u8>,
}

impl core::fmt::Debug for Image {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    /// Writes the image as a PNG file.
    pub fn save_png(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(path)?;
        let writer = io::BufWriter::new(file);
        let width = u32::try_from(self.width).map_err(io::Error::other)?;
        let height = u32::try_from(self.height).map_err(io::Error::other)?;
        let mut encoder = png::Encoder::new(writer, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer
            .write_image_data(&self.rgba)
            .map_err(io::Error::other)?;
        Ok(())
    }

    /// Reads a PNG file written by [`Self::save_png`].
    pub fn load_png(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let decoder = png::Decoder::new(io::BufReader::new(file));
        let mut reader = decoder.read_info().map_err(io::Error::other)?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| io::Error::other("too large"))?;
        let mut buffer = vec![0; size];
        let info = reader.next_frame(&mut buffer).map_err(io::Error::other)?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(io::Error::other("not 8-bit RGBA"));
        }
        buffer.truncate(info.buffer_size());
        Ok(Self {
            width: info.width as usize,
            height: info.height as usize,
            rgba: buffer,
        })
    }

    /// Whether `other` differs visibly from this image: another size, or
    /// more than `tolerance` of the pixels differing by more than 8 in some
    /// channel. Differences of the last bits of trigonometry move edge
    /// pixels by one or two levels and are not counted.
    pub fn differs(&self, other: &Self, tolerance: f64) -> bool {
        if self.width != other.width || self.height != other.height {
            return true;
        }
        let differing = self
            .rgba
            .chunks_exact(4)
            .zip(other.rgba.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 8))
            .count();
        let total = (self.width * self.height).max(1);
        differing as f64 / total as f64 > tolerance
    }
}

/// How to render.
#[derive(Clone, Debug)]
pub struct Options {
    /// The size of the screen in points.
    pub size: Vec2,
    /// Physical pixels per point.
    pub pixels_per_point: f32,
    /// How many frames to run before the one that is drawn, so that
    /// layout settles and animations end.
    pub frames: u32,
    /// Seconds between frames.
    pub frame_time: f64,
    /// The time of the first frame, in seconds: later than the last frame
    /// of an earlier render into the same context.
    pub start_time: f64,
    /// Where the pointer is, if anywhere.
    pub pointer: Option<Pos2>,
    /// Events given in the first frame only (key presses, text).
    pub events: Vec<Event>,
    /// What is behind everything.
    pub background: Color32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: Vec2::new(800.0, 600.0),
            pixels_per_point: 1.0,
            frames: 4,
            frame_time: 0.5,
            start_time: 0.0,
            pointer: None,
            events: Vec::new(),
            background: Color32::WHITE,
        }
    }
}

struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
    linear: bool,
}

/// Renders the frames `run` draws into `ctx` and returns the last one as
/// an image. `ctx` keeps its state between calls, so that a caller can set
/// fonts and the theme once.
pub fn render(ctx: &Context, options: &Options, mut run: impl FnMut(&Context)) -> Image {
    let mut textures: HashMap<TextureId, Texture> = HashMap::new();
    let mut shapes = Vec::new();
    let screen = Rect::from_min_size(Pos2::ZERO, options.size);
    let frames = options.frames.max(1);
    for frame in 0..=frames {
        let mut input = RawInput {
            screen_rect: Some(screen),
            time: Some(options.start_time + f64::from(frame) * options.frame_time),
            predicted_dt: options.frame_time as f32,
            focused: true,
            ..Default::default()
        };
        if let Some(viewport) = input.viewports.get_mut(&ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(options.pixels_per_point);
        }
        if let Some(pointer) = options.pointer {
            input.events.push(Event::PointerMoved(pointer));
        }
        if frame == 0 {
            input.events.extend(options.events.iter().cloned());
        }
        let output = ctx.run(input, &mut run);
        apply(&mut textures, &output.textures_delta);
        shapes = output.shapes;
        for id in &output.textures_delta.free {
            textures.remove(id);
        }
    }
    let primitives = ctx.tessellate(shapes, options.pixels_per_point);
    rasterize(&primitives, &textures, options)
}

fn apply(textures: &mut HashMap<TextureId, Texture>, delta: &egui::TexturesDelta) {
    for (id, image_delta) in &delta.set {
        let ImageData::Color(image) = &image_delta.image;
        let [width, height] = image.size;
        let linear = image_delta.options.magnification == TextureFilter::Linear;
        match image_delta.pos {
            None => {
                textures.insert(
                    *id,
                    Texture {
                        width,
                        height,
                        pixels: image.pixels.clone(),
                        linear,
                    },
                );
            }
            Some([x0, y0]) => {
                if let Some(texture) = textures.get_mut(id) {
                    for y in 0..height {
                        for x in 0..width {
                            let (tx, ty) = (x0 + x, y0 + y);
                            if tx < texture.width && ty < texture.height {
                                texture.pixels[ty * texture.width + tx] =
                                    image.pixels[y * width + x];
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Rgba([f32; 4]);

impl Rgba {
    fn of(color: Color32) -> Self {
        let [r, g, b, a] = color.to_array();
        Self([
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        ])
    }
}

fn sample(texture: &Texture, uv: Pos2) -> Rgba {
    let at = |x: isize, y: isize| -> Rgba {
        let x = x.clamp(0, texture.width as isize - 1) as usize;
        let y = y.clamp(0, texture.height as isize - 1) as usize;
        Rgba::of(texture.pixels[y * texture.width + x])
    };
    let fx = uv.x * texture.width as f32 - 0.5;
    let fy = uv.y * texture.height as f32 - 0.5;
    if !texture.linear {
        return at(fx.round() as isize, fy.round() as isize);
    }
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    );
    let mut out = [0.0; 4];
    for (i, value) in out.iter_mut().enumerate() {
        let top = a.0[i] + (b.0[i] - a.0[i]) * tx;
        let bottom = c.0[i] + (d.0[i] - c.0[i]) * tx;
        *value = top + (bottom - top) * ty;
    }
    Rgba(out)
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Whether the edge from `a` to `b` of a counter-clockwise triangle (in
/// screen coordinates, y down) is a top or a left edge: pixels exactly on
/// such an edge belong to the triangle, so that shared edges are drawn
/// once.
fn top_left(a: Pos2, b: Pos2) -> bool {
    let d = b - a;
    (d.y == 0.0 && d.x < 0.0) || d.y > 0.0
}

fn rasterize(
    primitives: &[ClippedPrimitive],
    textures: &HashMap<TextureId, Texture>,
    options: &Options,
) -> Image {
    let ppp = options.pixels_per_point;
    let width = (options.size.x * ppp).round().max(1.0) as usize;
    let height = (options.size.y * ppp).round().max(1.0) as usize;
    let background = Rgba::of(options.background);
    let mut buffer = vec![background; width * height];

    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        let clip = primitive.clip_rect;
        let clip_x0 = (clip.min.x * ppp).round().clamp(0.0, width as f32) as usize;
        let clip_y0 = (clip.min.y * ppp).round().clamp(0.0, height as f32) as usize;
        let clip_x1 = (clip.max.x * ppp).round().clamp(0.0, width as f32) as usize;
        let clip_y1 = (clip.max.y * ppp).round().clamp(0.0, height as f32) as usize;
        for triangle in mesh.indices.chunks_exact(3) {
            let vertex =
                |index: u32| -> Option<Vertex> { mesh.vertices.get(index as usize).copied() };
            let (Some(mut v0), Some(mut v1), Some(v2)) = (
                vertex(triangle[0]),
                vertex(triangle[1]),
                vertex(triangle[2]),
            ) else {
                continue;
            };
            for v in [&mut v0, &mut v1] {
                v.pos = Pos2::new(v.pos.x * ppp, v.pos.y * ppp);
            }
            let v2 = Vertex {
                pos: Pos2::new(v2.pos.x * ppp, v2.pos.y * ppp),
                ..v2
            };
            let mut area = edge(v0.pos, v1.pos, v2.pos);
            if area == 0.0 {
                continue;
            }
            if area < 0.0 {
                core::mem::swap(&mut v0, &mut v1);
                area = -area;
            }
            let min_x = v0
                .pos
                .x
                .min(v1.pos.x)
                .min(v2.pos.x)
                .floor()
                .max(clip_x0 as f32) as usize;
            let min_y = v0
                .pos
                .y
                .min(v1.pos.y)
                .min(v2.pos.y)
                .floor()
                .max(clip_y0 as f32) as usize;
            let max_x = (v0.pos.x.max(v1.pos.x).max(v2.pos.x).ceil() as usize).min(clip_x1);
            let max_y = (v0.pos.y.max(v1.pos.y).max(v2.pos.y).ceil() as usize).min(clip_y1);
            let (c0, c1, c2) = (Rgba::of(v0.color), Rgba::of(v1.color), Rgba::of(v2.color));
            let (tl0, tl1, tl2) = (
                top_left(v1.pos, v2.pos),
                top_left(v2.pos, v0.pos),
                top_left(v0.pos, v1.pos),
            );
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = edge(v1.pos, v2.pos, p);
                    let w1 = edge(v2.pos, v0.pos, p);
                    let w2 = edge(v0.pos, v1.pos, p);
                    let inside = |w: f32, tl: bool| w > 0.0 || (w == 0.0 && tl);
                    if !(inside(w0, tl0) && inside(w1, tl1) && inside(w2, tl2)) {
                        continue;
                    }
                    let (b0, b1, b2) = (w0 / area, w1 / area, w2 / area);
                    let uv = Pos2::new(
                        v0.uv.x * b0 + v1.uv.x * b1 + v2.uv.x * b2,
                        v0.uv.y * b0 + v1.uv.y * b1 + v2.uv.y * b2,
                    );
                    let texel = sample(texture, uv);
                    let pixel = &mut buffer[y * width + x];
                    let mut src = [0.0; 4];
                    for (i, value) in src.iter_mut().enumerate() {
                        let color = c0.0[i] * b0 + c1.0[i] * b1 + c2.0[i] * b2;
                        *value = color * texel.0[i];
                    }
                    let keep = 1.0 - src[3];
                    for (i, channel) in pixel.0.iter_mut().enumerate() {
                        *channel = src[i] + *channel * keep;
                    }
                }
            }
        }
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in &buffer {
        let alpha = pixel.0[3].clamp(0.0, 1.0);
        for i in 0..3 {
            // The background is opaque, so alpha is one and the colour is
            // already not premultiplied.
            let value = if alpha > 0.0 { pixel.0[i] / alpha } else { 0.0 };
            rgba.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        rgba.push((alpha * 255.0).round() as u8);
    }
    Image {
        width,
        height,
        rgba,
    }
}
