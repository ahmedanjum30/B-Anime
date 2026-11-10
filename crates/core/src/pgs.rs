//! PGS (Blu-ray "HDMV presentation graphics") subtitles: pictures, not text.
//!
//! Each display set describes what's on screen from its timestamp until the next one: a list of
//! bitmaps (run-length encoded, 256-colour YCbCr palette) placed on a virtual screen, usually
//! 1920×1080. They're decoded here into positioned PNGs the player lays over the video.

use std::collections::HashMap;

use base64::Engine;
use serde::Serialize;

const PDS: u8 = 0x14;
const ODS: u8 = 0x15;
const PCS: u8 = 0x16;
const END: u8 = 0x80;

/// Everything a PGS track shows, ready for the player.
#[derive(Debug, Serialize)]
pub struct Track {
    /// The virtual screen the images are placed on.
    pub width: u16,
    pub height: u16,
    pub events: Vec<Event>,
}

#[derive(Debug, Serialize)]
pub struct Event {
    /// Milliseconds.
    pub start: u64,
    pub end: u64,
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// A `data:image/png` URL.
    pub src: String,
}

struct Placement {
    object: u16,
    x: u16,
    y: u16,
    crop: Option<(u16, u16, u16, u16)>,
}

struct Composition {
    palette: u8,
    placements: Vec<Placement>,
}

#[derive(Default)]
struct Object {
    width: u16,
    height: u16,
    rle: Vec<u8>,
}

/// Feed it display sets in order with [`Decoder::feed`], then [`Decoder::finish`].
#[derive(Default)]
pub struct Decoder {
    width: u16,
    height: u16,
    palettes: HashMap<u8, Box<[[u8; 4]; 256]>>,
    objects: HashMap<u16, Object>,
    /// (time, images) per display set; an empty list clears the screen.
    shown: Vec<(u64, Vec<Image>)>,
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

impl Decoder {
    /// One display set (in MKV, one block): a run of segments ending with END.
    pub fn feed(&mut self, time: u64, mut data: &[u8]) {
        let mut composition: Option<Composition> = None;
        while data.len() >= 3 {
            let kind = data[0];
            let len = u16::from_be_bytes([data[1], data[2]]) as usize;
            let Some(body) = data.get(3..3 + len) else { break };
            data = &data[3 + len..];
            match kind {
                PCS => composition = self.composition(body),
                PDS => self.palette(body),
                ODS => self.object(body),
                END => {
                    if let Some(c) = composition.take() {
                        let images = self.render(&c);
                        self.shown.push((time, images));
                    }
                }
                _ => {}
            }
        }
        if let Some(c) = composition {
            let images = self.render(&c);
            self.shown.push((time, images));
        }
    }

    fn composition(&mut self, b: &[u8]) -> Option<Composition> {
        self.width = u16_at(b, 0)?;
        self.height = u16_at(b, 2)?;
        let state = *b.get(7)?;
        if state & 0x80 != 0 {
            // Epoch start: nothing from before carries over.
            self.objects.clear();
            self.palettes.clear();
        }
        let palette = *b.get(9)?;
        let count = *b.get(10)? as usize;
        let mut placements = Vec::with_capacity(count);
        let mut i = 11;
        for _ in 0..count {
            let object = u16_at(b, i)?;
            let cropped = *b.get(i + 3)? & 0x80 != 0;
            let (x, y) = (u16_at(b, i + 4)?, u16_at(b, i + 6)?);
            i += 8;
            let crop = if cropped {
                let c = (u16_at(b, i)?, u16_at(b, i + 2)?, u16_at(b, i + 4)?, u16_at(b, i + 6)?);
                i += 8;
                Some(c)
            } else {
                None
            };
            placements.push(Placement { object, x, y, crop });
        }
        Some(Composition { palette, placements })
    }

    fn palette(&mut self, b: &[u8]) {
        let Some(&id) = b.first() else { return };
        let entries = self.palettes.entry(id).or_insert_with(|| Box::new([[0; 4]; 256]));
        for e in b.get(2..).unwrap_or_default().chunks_exact(5) {
            entries[e[0] as usize] = rgba(e[1], e[2], e[3], e[4]);
        }
    }

    fn object(&mut self, b: &[u8]) {
        let (Some(id), Some(&flags)) = (u16_at(b, 0), b.get(3)) else { return };
        if flags & 0x80 != 0 {
            // First fragment: 3-byte data length, width, height, then RLE data.
            let (Some(w), Some(h)) = (u16_at(b, 7), u16_at(b, 9)) else { return };
            self.objects.insert(id, Object { width: w, height: h, rle: b.get(11..).unwrap_or_default().to_vec() });
        } else if let Some(o) = self.objects.get_mut(&id) {
            o.rle.extend_from_slice(b.get(4..).unwrap_or_default());
        }
    }

    fn render(&self, c: &Composition) -> Vec<Image> {
        let Some(palette) = self.palettes.get(&c.palette) else { return Vec::new() };
        c.placements
            .iter()
            .filter_map(|p| {
                let o = self.objects.get(&p.object)?;
                let mut pixels = decode_rle(&o.rle, o.width, o.height);
                let (mut w, mut h, mut x, mut y) = (o.width, o.height, p.x, p.y);
                if let Some((cx, cy, cw, ch)) = p.crop {
                    let (cw, ch) = (cw.min(w.saturating_sub(cx)), ch.min(h.saturating_sub(cy)));
                    let mut cropped = Vec::with_capacity(cw as usize * ch as usize);
                    for row in cy..cy + ch {
                        let start = row as usize * w as usize + cx as usize;
                        cropped.extend_from_slice(&pixels[start..start + cw as usize]);
                    }
                    (pixels, w, h, x, y) = (cropped, cw, ch, x + cx, y + cy);
                }
                if w == 0 || h == 0 {
                    return None;
                }
                if pixels.iter().all(|i| palette[*i as usize][3] == 0) {
                    return None;
                }
                Some(Image { x, y, w, h, src: png_url(&pixels, palette, w, h)? })
            })
            .collect()
    }

    pub fn finish(self) -> Track {
        let mut events: Vec<Event> = Vec::new();
        let shown = self.shown;
        for (i, (start, images)) in shown.iter().enumerate() {
            if images.is_empty() {
                continue;
            }
            let end = shown.get(i + 1).map(|(t, _)| *t).unwrap_or(start + 5000).max(start + 1);
            // The same picture sent again (refresh points): extend instead of repeating.
            if let Some(prev) = events.last_mut()
                && prev.end == *start
                && prev.images == *images
            {
                prev.end = end;
                continue;
            }
            events.push(Event { start: *start, end, images: images.clone() });
        }
        Track { width: self.width.max(1), height: self.height.max(1), events }
    }
}

/// BT.709 limited-range YCbCr (as stored) to RGBA.
fn rgba(y: u8, cr: u8, cb: u8, a: u8) -> [u8; 4] {
    let y = (y as f32 - 16.0) * 1.164;
    let (cb, cr) = (cb as f32 - 128.0, cr as f32 - 128.0);
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(y + 1.793 * cr), c(y - 0.213 * cb - 0.533 * cr), c(y + 2.112 * cb), a]
}

/// PGS run-length data to palette indices, row by row.
fn decode_rle(rle: &[u8], width: u16, height: u16) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h];
    let (mut x, mut y, mut i) = (0usize, 0usize, 0usize);
    let next = |i: &mut usize| -> Option<u8> {
        let b = *rle.get(*i)?;
        *i += 1;
        Some(b)
    };
    while y < h {
        let Some(b) = next(&mut i) else { break };
        let (count, color) = if b != 0 {
            (1, b)
        } else {
            let Some(f) = next(&mut i) else { break };
            match f >> 6 {
                _ if f == 0 => {
                    // End of line.
                    x = 0;
                    y += 1;
                    continue;
                }
                0 => ((f & 0x3F) as usize, 0),
                1 => ((((f & 0x3F) as usize) << 8) | next(&mut i).unwrap_or(0) as usize, 0),
                2 => ((f & 0x3F) as usize, next(&mut i).unwrap_or(0)),
                _ => {
                    let n = (((f & 0x3F) as usize) << 8) | next(&mut i).unwrap_or(0) as usize;
                    (n, next(&mut i).unwrap_or(0))
                }
            }
        };
        let n = count.min(w.saturating_sub(x));
        let row = y * w;
        out[row + x..row + x + n].fill(color);
        x += n;
    }
    out
}

/// A palette PNG (PGS pictures have at most 256 colours): a fraction of the size of RGBA.
fn png_url(indices: &[u8], palette: &[[u8; 4]; 256], w: u16, h: u16) -> Option<String> {
    let used = *indices.iter().max()? as usize + 1;
    let colors: Vec<u8> = palette[..used].iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    let alpha: Vec<u8> = palette[..used].iter().map(|c| c[3]).collect();
    let mut bytes = Vec::with_capacity(indices.len() / 8);
    {
        let mut enc = png::Encoder::new(&mut bytes, w as u32, h as u32);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(colors);
        enc.set_trns(alpha);
        enc.set_compression(png::Compression::Balanced);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(indices).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut s = vec![kind];
        s.extend_from_slice(&(body.len() as u16).to_be_bytes());
        s.extend_from_slice(body);
        s
    }

    #[test]
    fn decodes_a_display_set() {
        // 1920x1080, epoch start, palette 0, one object at (100, 900).
        let pcs = [7, 128, 4, 56, 0x10, 0, 1, 0x80, 0, 0, 1, 0, 0, 0, 0, 0, 100, 3, 132];
        // Palette: index 1 = opaque white (Y 235, Cr/Cb 128).
        let pds = [0, 0, 1, 235, 128, 128, 255];
        // Object 0, 4x2: row 1 = four white pixels, row 2 = two transparent then two white.
        let rle = [0x00, 0x84, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x01, 0x00, 0x00];
        let mut ods = vec![0, 0, 0, 0xC0, 0, 0, (rle.len() + 4) as u8, 0, 4, 0, 2];
        ods.extend_from_slice(&rle);
        let mut set = segment(PCS, &pcs);
        set.extend(segment(PDS, &pds));
        set.extend(segment(ODS, &ods));
        set.extend(segment(END, &[]));
        let clear = [segment(PCS, &[7, 128, 4, 56, 0x10, 0, 2, 0, 0, 0, 0]), segment(END, &[])].concat();

        let mut d = Decoder::default();
        d.feed(1000, &set);
        d.feed(2500, &clear);
        let track = d.finish();
        assert_eq!((track.width, track.height), (1920, 1080));
        assert_eq!(track.events.len(), 1);
        let e = &track.events[0];
        assert_eq!((e.start, e.end), (1000, 2500));
        assert_eq!((e.images[0].x, e.images[0].y, e.images[0].w, e.images[0].h), (100, 900, 4, 2));
        assert!(e.images[0].src.starts_with("data:image/png;base64,"));
        assert_eq!(decode_rle(&rle, 4, 2), vec![1, 1, 1, 1, 0, 0, 1, 1]);
        assert_eq!(rgba(235, 128, 128, 255), [255, 255, 255, 255]);
    }
}
