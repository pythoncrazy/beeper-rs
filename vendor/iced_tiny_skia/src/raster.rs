use crate::core::image as raster;
use crate::core::{Rectangle, Size};
use crate::graphics;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::collections::hash_map;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
        }
    }

    pub fn load(
        &self,
        handle: &raster::Handle,
    ) -> Result<raster::Allocation, raster::Error> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle)?;

        #[allow(unsafe_code)]
        Ok(unsafe {
            raster::allocate(handle, Size::new(image.width(), image.height()))
        })
    }

    pub fn dimensions(&self, handle: &raster::Handle) -> Option<Size<u32>> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle).ok()?;

        Some(Size::new(image.width(), image.height()))
    }

    pub fn draw(
        &mut self,
        handle: &raster::Handle,
        filter_method: raster::FilterMethod,
        bounds: Rectangle,
        opacity: f32,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        transform: tiny_skia::Transform,
        clip_mask: Option<&tiny_skia::Mask>,
    ) {
        let mut cache = self.cache.borrow_mut();

        let quality = match filter_method {
            raster::FilterMethod::Linear => tiny_skia::FilterQuality::Bilinear,
            raster::FilterMethod::Nearest => tiny_skia::FilterQuality::Nearest,
        };

        // An image that is not turned draws at whole pixels from a copy at the
        // size that it shows, which is made once: a scaled image or one at a
        // fraction of a pixel otherwise goes through the filter of tiny-skia
        // at each pixel of each frame.
        if let Some((x, y, size)) = snapped(bounds, transform) {
            if let Some(image) = cache.scaled(handle, size, quality) {
                crate::blit::draw(pixels, x, y, image, opacity, clip_mask);
            }
            return;
        }

        let Ok(image) = cache.allocate(handle) else {
            return;
        };

        let width_scale = bounds.width / image.width() as f32;
        let height_scale = bounds.height / image.height() as f32;

        // The image goes at the exact position of `bounds`: an integer offset in
        // the pixels of the image moved a scaled up image up to a scale to the
        // left and up, out of its damage.
        let transform = transform
            .pre_translate(bounds.x, bounds.y)
            .pre_scale(width_scale, height_scale);

        pixels.draw_pixmap(
            0,
            0,
            image,
            &tiny_skia::PixmapPaint {
                quality,
                opacity,
                ..Default::default()
            },
            transform,
            clip_mask,
        );
    }

    pub fn trim_cache(&mut self) {
        self.cache.borrow_mut().trim();
    }
}

/// The largest copy of an image at the size that it shows, in pixels.
const MAX_SCALED: u32 = 4096 * 4096;

/// The whole pixels of `bounds` under `transform` without a turn or a skew:
/// the position of the top left corner, and the size.
fn snapped(
    bounds: Rectangle,
    transform: tiny_skia::Transform,
) -> Option<(i32, i32, (u32, u32))> {
    let t = transform;
    if t.kx != 0.0 || t.ky != 0.0 || t.sx <= 0.0 || t.sy <= 0.0 {
        return None;
    }
    // Both edges round to the nearest pixel, so the image stays in the pixels
    // that its bounds touch, which its damage covers.
    let x0 = (bounds.x * t.sx + t.tx).round();
    let y0 = (bounds.y * t.sy + t.ty).round();
    let x1 = ((bounds.x + bounds.width) * t.sx + t.tx).round();
    let y1 = ((bounds.y + bounds.height) * t.sy + t.ty).round();
    let (w, h) = (x1 - x0, y1 - y0);
    let fits = w >= 1.0 && h >= 1.0 && w * h <= MAX_SCALED as f32;
    // Too far out for an `i32` is too far to show.
    let near = x0.abs() < 1e9 && y0.abs() < 1e9;
    (fits && near).then(|| (x0 as i32, y0 as i32, (w as u32, h as u32)))
}

#[derive(Debug, Default)]
struct Cache {
    entries: FxHashMap<raster::Id, Option<Entry>>,
    hits: FxHashSet<raster::Id>,
    /// The copies of the images at the sizes that they showed in the last
    /// frame, by filter.
    scaled: FxHashMap<(raster::Id, (u32, u32), bool), tiny_skia::Pixmap>,
    scaled_hits: FxHashSet<(raster::Id, (u32, u32), bool)>,
}

impl Cache {
    pub fn allocate(
        &mut self,
        handle: &raster::Handle,
    ) -> Result<tiny_skia::PixmapRef<'_>, raster::Error> {
        let id = handle.id();

        if let hash_map::Entry::Vacant(entry) = self.entries.entry(id) {
            let image = match graphics::image::load(handle) {
                Ok(image) => image,
                Err(error) => {
                    let _ = entry.insert(None);

                    return Err(error);
                }
            };

            if image.width() == 0 || image.height() == 0 {
                return Err(raster::Error::Empty);
            }

            let mut buffer =
                vec![0u32; image.width() as usize * image.height() as usize];

            for (i, pixel) in image.pixels().enumerate() {
                let [r, g, b, a] = pixel.0;

                buffer[i] = bytemuck::cast(
                    tiny_skia::ColorU8::from_rgba(b, g, r, a).premultiply(),
                );
            }

            let _ = entry.insert(Some(Entry {
                width: image.width(),
                height: image.height(),
                pixels: buffer,
            }));
        }

        let _ = self.hits.insert(id);

        Ok(self
            .entries
            .get(&id)
            .unwrap()
            .as_ref()
            .map(|entry| {
                tiny_skia::PixmapRef::from_bytes(
                    bytemuck::cast_slice(&entry.pixels),
                    entry.width,
                    entry.height,
                )
                .expect("Build pixmap from image bytes")
            })
            .expect("Image should be allocated"))
    }

    /// The image of `handle` at `size`: the image itself at its own size, or
    /// a copy drawn with `quality`.
    fn scaled(
        &mut self,
        handle: &raster::Handle,
        size: (u32, u32),
        quality: tiny_skia::FilterQuality,
    ) -> Option<tiny_skia::PixmapRef<'_>> {
        let (width, height) = {
            let image = self.allocate(handle).ok()?;
            (image.width(), image.height())
        };
        if (width, height) == size {
            return self.allocate(handle).ok();
        }
        let key = (
            handle.id(),
            size,
            quality == tiny_skia::FilterQuality::Nearest,
        );
        let _ = self.scaled_hits.insert(key);
        if !self.scaled.contains_key(&key) {
            let entry = self.entries.get(&key.0)?.as_ref()?;
            let image = tiny_skia::PixmapRef::from_bytes(
                bytemuck::cast_slice(&entry.pixels),
                entry.width,
                entry.height,
            )?;
            let mut copy = tiny_skia::Pixmap::new(size.0, size.1)?;
            copy.draw_pixmap(
                0,
                0,
                image,
                &tiny_skia::PixmapPaint {
                    quality,
                    ..Default::default()
                },
                tiny_skia::Transform::from_scale(
                    size.0 as f32 / width as f32,
                    size.1 as f32 / height as f32,
                ),
                None,
            );
            let _ = self.scaled.insert(key, copy);
        }
        self.scaled.get(&key).map(tiny_skia::Pixmap::as_ref)
    }

    fn trim(&mut self) {
        self.entries.retain(|key, _| self.hits.contains(key));
        self.hits.clear();
        self.scaled.retain(|key, _| self.scaled_hits.contains(key));
        self.scaled_hits.clear();
    }
}

#[derive(Debug)]
struct Entry {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}
