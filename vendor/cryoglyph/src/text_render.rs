use crate::{
    ColorMode, FontSystem, GlyphDetails, GlyphToRender, GpuCacheStatus, PrepareError, RenderError,
    SwashCache, SwashContent, TextArea, TextAtlas, Viewport,
};
use std::{num::NonZeroU64, slice};
use wgpu::util::StagingBelt;
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, COPY_BUFFER_ALIGNMENT, CommandEncoder,
    DepthStencilState, Device, Extent3d, MultisampleState, Origin3d, Queue, RenderPass,
    RenderPipeline, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
};

/// A text renderer that uses cached glyphs to render text into an existing render pass.
pub struct TextRenderer {
    staging_belt: StagingBelt,
    vertex_buffer: Buffer,
    vertex_buffer_size: u64,
    pipeline: RenderPipeline,
    glyph_vertices: Vec<GlyphToRender>,
    glyphs_to_render: u32,
    /// The atlas glyphs of `glyph_vertices`.
    glyphs: Vec<(cosmic_text::CacheKey, ContentType)>,
    /// The atlas generation after the last `prepare`, if it succeeded.
    generation: Option<u64>,
}

impl TextRenderer {
    /// Creates a new `TextRenderer`.
    pub fn new(
        atlas: &mut TextAtlas,
        device: &Device,
        multisample: MultisampleState,
        depth_stencil: Option<DepthStencilState>,
    ) -> Self {
        let vertex_buffer_size = next_copy_buffer_size(4096);
        let vertex_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon vertices"),
            size: vertex_buffer_size,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = atlas.get_or_create_pipeline(device, multisample, depth_stencil);

        Self {
            staging_belt: StagingBelt::new(vertex_buffer_size),
            vertex_buffer,
            vertex_buffer_size,
            pipeline,
            glyph_vertices: Vec::new(),
            glyphs_to_render: 0,
            glyphs: Vec::new(),
            generation: None,
        }
    }

    /// Prepares all of the provided text areas for rendering.
    pub fn prepare_with_depth<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
        mut metadata_to_depth: impl FnMut(usize) -> f32,
    ) -> Result<(), PrepareError> {
        self.staging_belt.recall();
        self.glyph_vertices.clear();
        self.glyphs.clear();
        self.generation = None;

        let resolution = viewport.resolution();

        for text_area in text_areas {
            let bounds_min_x = text_area.bounds.left.max(0);
            let bounds_min_y = text_area.bounds.top.max(0);
            let bounds_max_x = text_area.bounds.right.min(resolution.width as i32);
            let bounds_max_y = text_area.bounds.bottom.min(resolution.height as i32);

            let is_run_visible = |run: &cosmic_text::LayoutRun| {
                let start_y_physical = (text_area.top + (run.line_top * text_area.scale)) as i32;
                let end_y_physical = start_y_physical + (run.line_height * text_area.scale) as i32;

                start_y_physical <= text_area.bounds.bottom
                    && text_area.bounds.top <= end_y_physical
            };

            let layout_runs = text_area
                .buffer
                .layout_runs()
                .skip_while(|run| !is_run_visible(run))
                .take_while(is_run_visible);

            for run in layout_runs {
                for glyph in run.glyphs.iter() {
                    let (cache_key, glyph_x, glyph_y) = place(
                        glyph,
                        run.line_y,
                        (text_area.left, text_area.top),
                        text_area.scale,
                    );

                    let details = if let Some(details) =
                        atlas.mask_atlas.glyph_cache.get(&cache_key)
                    {
                        atlas.mask_atlas.glyphs_in_use.insert(cache_key);
                        details
                    } else if let Some(details) = atlas.color_atlas.glyph_cache.get(&cache_key) {
                        atlas.color_atlas.glyphs_in_use.insert(cache_key);
                        details
                    } else {
                        let Some(image) =
                            cache.get_image_uncached(font_system, cache_key)
                        else {
                            continue;
                        };

                        let content_type = match image.content {
                            SwashContent::Color => ContentType::Color,
                            SwashContent::Mask => ContentType::Mask,
                            SwashContent::SubpixelMask => {
                                // Not implemented yet, but don't panic if this happens.
                                ContentType::Mask
                            }
                        };

                        let width = image.placement.width as usize;
                        let height = image.placement.height as usize;

                        let should_rasterize = width > 0 && height > 0;

                        let (gpu_cache, atlas_id, inner) = if should_rasterize {
                            let mut inner = atlas.inner_for_content_mut(content_type);

                            // Find a position in the packer
                            let allocation = loop {
                                match inner.try_allocate(width, height) {
                                    Some(a) => break a,
                                    None => {
                                        if !atlas.grow(
                                            device,
                                            queue,
                                            font_system,
                                            cache,
                                            content_type,
                                        ) {
                                            return Err(PrepareError::AtlasFull);
                                        }

                                        inner = atlas.inner_for_content_mut(content_type);
                                    }
                                }
                            };
                            let atlas_min = allocation.rectangle.min;

                            queue.write_texture(
                                TexelCopyTextureInfo {
                                    texture: &inner.texture,
                                    mip_level: 0,
                                    origin: Origin3d {
                                        x: atlas_min.x as u32,
                                        y: atlas_min.y as u32,
                                        z: 0,
                                    },
                                    aspect: TextureAspect::All,
                                },
                                &image.data,
                                TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(width as u32 * inner.num_channels() as u32),
                                    rows_per_image: None,
                                },
                                Extent3d {
                                    width: width as u32,
                                    height: height as u32,
                                    depth_or_array_layers: 1,
                                },
                            );

                            (
                                GpuCacheStatus::InAtlas {
                                    x: atlas_min.x as u16,
                                    y: atlas_min.y as u16,
                                    content_type,
                                },
                                Some(allocation.id),
                                inner,
                            )
                        } else {
                            let inner = &mut atlas.color_atlas;
                            (GpuCacheStatus::SkipRasterization, None, inner)
                        };

                        inner.glyphs_in_use.insert(cache_key);
                        // Insert the glyph into the cache and return the details reference
                        inner.glyph_cache.get_or_insert(cache_key, || GlyphDetails {
                            width: image.placement.width as u16,
                            height: image.placement.height as u16,
                            gpu_cache,
                            atlas_id,
                            top: image.placement.top as i16,
                            left: image.placement.left as i16,
                        })
                    };

                    let mut x = glyph_x + details.left as i32;
                    let mut y = glyph_y - details.top as i32;

                    let (mut atlas_x, mut atlas_y, content_type) = match details.gpu_cache {
                        GpuCacheStatus::InAtlas { x, y, content_type } => (x, y, content_type),
                        GpuCacheStatus::SkipRasterization => continue,
                    };

                    let mut width = details.width as i32;
                    let mut height = details.height as i32;

                    // Starts beyond right edge or ends beyond left edge
                    let max_x = x + width;
                    if x > bounds_max_x || max_x < bounds_min_x {
                        continue;
                    }

                    // Starts beyond bottom edge or ends beyond top edge
                    let max_y = y + height;
                    if y > bounds_max_y || max_y < bounds_min_y {
                        continue;
                    }

                    // Clip left ege
                    if x < bounds_min_x {
                        let right_shift = bounds_min_x - x;

                        x = bounds_min_x;
                        width = max_x - bounds_min_x;
                        atlas_x += right_shift as u16;
                    }

                    // Clip right edge
                    if x + width > bounds_max_x {
                        width = bounds_max_x - x;
                    }

                    // Clip top edge
                    if y < bounds_min_y {
                        let bottom_shift = bounds_min_y - y;

                        y = bounds_min_y;
                        height = max_y - bounds_min_y;
                        atlas_y += bottom_shift as u16;
                    }

                    // Clip bottom edge
                    if y + height > bounds_max_y {
                        height = bounds_max_y - y;
                    }

                    let color = match glyph.color_opt {
                        Some(some) => some,
                        None => text_area.default_color,
                    };

                    let depth = metadata_to_depth(glyph.metadata);
                    self.glyphs.push((cache_key, content_type));

                    self.glyph_vertices.push(GlyphToRender {
                        pos: [x, y],
                        dim: [width as u16, height as u16],
                        uv: [atlas_x, atlas_y],
                        color: color.0,
                        content_type_with_srgb: [
                            content_type as u16,
                            match atlas.color_mode {
                                ColorMode::Accurate => TextColorConversion::ConvertToLinear,
                                ColorMode::Web => TextColorConversion::None,
                            } as u16,
                        ],
                        depth,
                    });
                }
            }
        }

        self.glyphs_to_render = self.glyph_vertices.len() as u32;

        let will_render = !self.glyph_vertices.is_empty();
        if !will_render {
            self.generation = Some(crate::text_atlas::generation());
            return Ok(());
        }

        let vertices = self.glyph_vertices.as_slice();
        let vertices_raw = unsafe {
            slice::from_raw_parts(
                vertices as *const _ as *const u8,
                std::mem::size_of_val(vertices),
            )
        };

        if self.vertex_buffer_size >= vertices_raw.len() as u64 {
            self.staging_belt
                .write_buffer(
                    encoder,
                    &self.vertex_buffer,
                    0,
                    NonZeroU64::new(vertices_raw.len() as u64).expect("Non-empty vertices"),
                    device,
                )
                .copy_from_slice(vertices_raw);
        } else {
            self.vertex_buffer.destroy();

            let (buffer, buffer_size) = create_oversized_buffer(
                device,
                Some("glyphon vertices"),
                vertices_raw,
                BufferUsages::VERTEX | BufferUsages::COPY_DST,
            );

            self.vertex_buffer = buffer;
            self.vertex_buffer_size = buffer_size;

            self.staging_belt.finish();
            self.staging_belt = StagingBelt::new(buffer_size);
        }

        self.staging_belt.finish();
        self.generation = Some(crate::text_atlas::generation());

        Ok(())
    }

    /// Keeps what the last `prepare` prepared for one more frame, if no atlas has evicted a glyph since: the glyphs
    /// are where it found them, and marking them in use keeps the rest of the frame from evicting them. Returns
    /// `false` when the text needs a `prepare`.
    pub fn reuse(&self, atlas: &mut TextAtlas) -> bool {
        if self.generation != Some(crate::text_atlas::generation()) {
            return false;
        }
        for &(key, content_type) in &self.glyphs {
            let _ = atlas.inner_for_content_mut(content_type).glyphs_in_use.insert(key);
        }
        true
    }

    pub fn prepare<'a>(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        font_system: &mut FontSystem,
        atlas: &mut TextAtlas,
        viewport: &Viewport,
        text_areas: impl IntoIterator<Item = TextArea<'a>>,
        cache: &mut SwashCache,
    ) -> Result<(), PrepareError> {
        self.prepare_with_depth(
            device,
            queue,
            encoder,
            font_system,
            atlas,
            viewport,
            text_areas,
            cache,
            zero_depth,
        )
    }

    /// Renders all layouts that were previously provided to `prepare`.
    pub fn render(
        &self,
        atlas: &TextAtlas,
        viewport: &Viewport,
        pass: &mut RenderPass<'_>,
    ) -> Result<(), RenderError> {
        if self.glyphs_to_render == 0 {
            return Ok(());
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &atlas.bind_group, &[]);
        pass.set_bind_group(1, &viewport.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..4, 0..self.glyphs_to_render);

        Ok(())
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ContentType {
    Color = 0,
    Mask = 1,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TextColorConversion {
    None = 0,
    ConvertToLinear = 1,
}

fn next_copy_buffer_size(size: u64) -> u64 {
    let align_mask = COPY_BUFFER_ALIGNMENT - 1;
    ((size.next_power_of_two() + align_mask) & !align_mask).max(COPY_BUFFER_ALIGNMENT)
}

fn create_oversized_buffer(
    device: &Device,
    label: Option<&str>,
    contents: &[u8],
    usage: BufferUsages,
) -> (Buffer, u64) {
    let size = next_copy_buffer_size(contents.len() as u64);
    let buffer = device.create_buffer(&BufferDescriptor {
        label,
        size,
        usage,
        mapped_at_creation: true,
    });
    buffer.slice(..).get_mapped_range_mut()[..contents.len()].copy_from_slice(contents);
    buffer.unmap();
    (buffer, size)
}

fn zero_depth(_: usize) -> f32 {
    0f32
}

/// beeper-rs: the cache key and the pixel of `glyph` on the baseline `line_y` of a text at `(left, top)`.
/// cosmic-text keeps a baseline on whole pixels (its hinting in the Y axis), which suits text. A glyph of a font
/// without hinting (the icon font: Desktop draws icons as SVGs, wherever their boxes land) takes its place to a
/// quarter of a pixel instead, so an icon is not up to 1.5 px above its place on a screen at scale 1.
pub fn place(glyph: &cosmic_text::LayoutGlyph, line_y: f32, (left, top): (f32, f32), scale: f32) -> (cosmic_text::CacheKey, i32, i32) {
    let physical = glyph.physical((left, top), scale);
    if !glyph.cache_key_flags.contains(cosmic_text::CacheKeyFlags::DISABLE_HINTING) {
        return (physical.cache_key, physical.x, physical.y + (line_y * scale).round() as i32);
    }
    // swash draws a glyph `y_bin` above its baseline, so the baseline is the next pixel down.
    let y = (glyph.y - glyph.font_size * glyph.y_offset + line_y) * scale + top;
    let (whole, y_bin) = cosmic_text::SubpixelBin::new(y.ceil() - y);
    (cosmic_text::CacheKey { y_bin, ..physical.cache_key }, physical.x, y.ceil() as i32 - whole)
}
