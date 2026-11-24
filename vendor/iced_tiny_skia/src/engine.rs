use crate::Primitive;
use crate::core::renderer::Quad;
use crate::core::{
    Background, Color, Gradient, Rectangle, Size, Transformation, Vector,
};
use crate::graphics::{Image, Text};
use crate::text;

#[derive(Debug)]
pub struct Engine {
    text_pipeline: text::Pipeline,

    #[cfg(feature = "image")]
    pub(crate) raster_pipeline: crate::raster::Pipeline,
    #[cfg(feature = "svg")]
    pub(crate) vector_pipeline: crate::vector::Pipeline,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            text_pipeline: text::Pipeline::new(),
            #[cfg(feature = "image")]
            raster_pipeline: crate::raster::Pipeline::new(),
            #[cfg(feature = "svg")]
            vector_pipeline: crate::vector::Pipeline::new(),
        }
    }

    pub fn draw_quad(
        &mut self,
        quad: &Quad,
        background: &Background,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        let physical_bounds = quad.bounds * transformation;

        if !clip_bounds.intersects(&physical_bounds) {
            return;
        }

        let transform = into_transform(transformation);

        // Make sure the border radius is not larger than the bounds
        let border_width = quad
            .border
            .width
            .min(quad.bounds.width / 2.0)
            .min(quad.bounds.height / 2.0);

        let mut fill_border_radius = <[f32; 4]>::from(quad.border.radius);

        for radius in &mut fill_border_radius {
            *radius = (*radius)
                .min(quad.bounds.width / 2.0)
                .min(quad.bounds.height / 2.0);
        }

        let path = rounded_rectangle(quad.bounds, fill_border_radius);

        let shadow = quad.shadow;

        if shadow.color.a > 0.0 {
            let shadow_bounds = Rectangle {
                x: quad.bounds.x + shadow.offset.x - shadow.blur_radius,
                y: quad.bounds.y + shadow.offset.y - shadow.blur_radius,
                width: quad.bounds.width + shadow.blur_radius * 2.0,
                height: quad.bounds.height + shadow.blur_radius * 2.0,
            } * transformation;

            let radii = fill_border_radius
                .into_iter()
                .map(|radius| radius * transformation.scale_factor())
                .collect::<Vec<_>>();
            let (x, y, width, height) = (
                shadow_bounds.x as u32,
                shadow_bounds.y as u32,
                shadow_bounds.width as u32,
                shadow_bounds.height as u32,
            );
            let half_width = physical_bounds.width / 2.0;
            let half_height = physical_bounds.height / 2.0;

            // Only the pixels in the clip bounds change. The fill of an opaque
            // quad covers its inside, so the shadow there is not computed.
            let (x0, y0) = (
                x.max(clip_bounds.x.max(0.0) as u32),
                y.max(clip_bounds.y.max(0.0) as u32),
            );
            let (x1, y1) = (
                (x + width).min((clip_bounds.x + clip_bounds.width).ceil() as u32),
                (y + height).min((clip_bounds.y + clip_bounds.height).ceil() as u32),
            );
            let opaque = matches!(background, Background::Color(c) if c.a >= 1.0);
            let inset = radii.iter().copied().fold(0.0, f32::max) + 1.0;
            let inside = |start: f32, end: f32, lo: u32, hi: u32| {
                let (lo, hi) = (lo as f32, hi as f32);
                ((start + inset).ceil().clamp(lo, hi) as u32, (end - inset).floor().clamp(lo, hi) as u32)
            };
            let (cx0, cx1) = inside(physical_bounds.x, physical_bounds.x + physical_bounds.width, x0, x1);
            let (cy0, cy1) = inside(physical_bounds.y, physical_bounds.y + physical_bounds.height, y0, y1);
            let parts = if opaque && cx0 < cx1 && cy0 < cy1 {
                vec![(x0, y0, x1, cy0), (x0, cy1, x1, y1), (x0, cy0, cx0, cy1), (cx1, cy0, x1, cy1)]
            } else {
                vec![(x0, y0, x1, y1)]
            };

            let scale = transformation.scale_factor();
            let blur = shadow.blur_radius * scale;
            let center = (
                physical_bounds.x + shadow.offset.x * scale + half_width,
                physical_bounds.y + shadow.offset.y * scale + half_height,
            );
            let size = tiny_skia::Size::from_wh(half_width, half_height);
            let color_at = |x: f32, y: f32, size: tiny_skia::Size| {
                let distance = rounded_box_sdf(
                    Vector::new(x - center.0, y - center.1),
                    size,
                    &radii,
                )
                .max(0.0);
                let mut color = into_color(shadow.color);
                color.apply_opacity(1.0 - smoothstep(-blur, blur, distance));

                color.to_color_u8().premultiply()
            };
            // beeper-rs: beside a straight side of the quad, a pixel is as
            // far from it as every pixel of its row or its column there, so
            // one of them is computed: the shadow of the open chat's card
            // was a fifth of a frame of the whole window.
            let corner = radii.iter().copied().fold(0.0, f32::max);
            let beside = |at: f32, center: f32, half: f32| {
                (at - center).abs() <= half - corner
            };

            for (px0, py0, px1, py1) in parts {
                let Some(size) = size.filter(|_| px0 < px1 && py0 < py1) else {
                    continue;
                };
                let width = (px1 - px0) as usize;
                let mut colors =
                    Vec::with_capacity(width * (py1 - py0) as usize);
                let mut above = false;

                for y in (py0..py1).map(|y| y as f32) {
                    let straight = beside(y, center.1, half_height);

                    if straight && above {
                        colors.extend_from_within(colors.len() - width..);
                        continue;
                    }
                    above = straight;

                    let mut column = None;
                    colors.extend((px0..px1).map(|x| x as f32).map(|x| {
                        if beside(x, center.0, half_width) {
                            *column.get_or_insert_with(|| color_at(x, y, size))
                        } else {
                            color_at(x, y, size)
                        }
                    }));
                }

                if let Some(pixmap) =
                    tiny_skia::IntSize::from_wh(px1 - px0, py1 - py0)
                        .and_then(|size| {
                            tiny_skia::Pixmap::from_vec(
                                bytemuck::cast_vec(colors),
                                size,
                            )
                        })
                {
                    crate::blit::draw(
                        pixels,
                        px0 as i32,
                        py0 as i32,
                        pixmap.as_ref(),
                        1.0,
                        Some(clip_mask),
                    );
                }
            }
        }

        let clip_mask = (!physical_bounds.is_within(&clip_bounds))
            .then_some(clip_mask as &_);

        // The mask hides the fill outside the clip bounds, but tiny-skia
        // still fills all pixels of the path. When the clip bounds, a pixel
        // larger, miss the rounded corners, the fill in them is a rectangle.
        let path = clip_mask
            .and_then(|_| {
                let near = clip_bounds.expand(1.0)
                    * Transformation::scale(1.0 / transformation.scale_factor());
                let shown = quad.bounds.intersection(&near)?;
                let [tl, tr, br, bl] = fill_border_radius;
                let (b, right, bottom) = (
                    quad.bounds,
                    quad.bounds.x + quad.bounds.width,
                    quad.bounds.y + quad.bounds.height,
                );
                let corners = [
                    (b.x, b.y, tl),
                    (right - tr, b.y, tr),
                    (right - br, bottom - br, br),
                    (b.x, bottom - bl, bl),
                ];
                corners
                    .iter()
                    .all(|&(x, y, r)| {
                        !Rectangle::new(crate::core::Point::new(x, y), Size::new(r, r))
                            .intersects(&shown)
                    })
                    .then(|| rounded_rectangle(shown, [0.0; 4]))
            })
            .unwrap_or(path);

        // beeper-rs: a quad with a clear fill or border has nothing to
        // draw there, but tiny-skia ran its whole path through the pipeline.
        if !matches!(background, Background::Color(c) if c.a <= 0.0) {
            let paint = tiny_skia::Paint {
                shader: match background {
                Background::Color(color) => {
                    tiny_skia::Shader::SolidColor(into_color(*color))
                }
                Background::Gradient(Gradient::Linear(linear)) => {
                    let (start, end) =
                        linear.angle.to_distance(&quad.bounds);
    
                    let stops: Vec<tiny_skia::GradientStop> = linear
                        .stops
                        .into_iter()
                        .flatten()
                        .map(|stop| {
                            tiny_skia::GradientStop::new(
                                stop.offset,
                                tiny_skia::Color::from_rgba(
                                    stop.color.b,
                                    stop.color.g,
                                    stop.color.r,
                                    stop.color.a,
                                )
                                .expect("Create color"),
                            )
                        })
                        .collect();
    
                    tiny_skia::LinearGradient::new(
                        tiny_skia::Point {
                            x: start.x,
                            y: start.y,
                        },
                        tiny_skia::Point { x: end.x, y: end.y },
                        if stops.is_empty() {
                            vec![tiny_skia::GradientStop::new(
                                0.0,
                                tiny_skia::Color::BLACK,
                            )]
                        } else {
                            stops
                        },
                        tiny_skia::SpreadMode::Pad,
                        tiny_skia::Transform::identity(),
                    )
                    .expect("Create linear gradient")
                }
            },
                anti_alias: true,
                ..tiny_skia::Paint::default()
            };
            let rows = Rows::new(background, quad.bounds, transformation);
            let radii =
                fill_border_radius.map(|r| r * transformation.scale_factor());
            let filled = rows.is_some_and(|rows| {
                fill_rows(
                    pixels,
                    physical_bounds,
                    radii,
                    &rows,
                    (&paint, transformation),
                    clip_bounds,
                    clip_mask,
                )
            });

            if !filled {
                pixels.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::EvenOdd,
                    transform,
                    clip_mask,
                );
            }
        }

        if border_width > 0.0 && quad.border.color.a > 0.0 {
            // Border path is offset by half the border width
            let border_bounds = Rectangle {
                x: quad.bounds.x + border_width / 2.0,
                y: quad.bounds.y + border_width / 2.0,
                width: quad.bounds.width - border_width,
                height: quad.bounds.height - border_width,
            };

            // Make sure the border radius is correct
            let mut border_radius = <[f32; 4]>::from(quad.border.radius);
            let mut is_simple_border = true;

            for radius in &mut border_radius {
                *radius = if *radius == 0.0 {
                    // Path should handle this fine
                    0.0
                } else if *radius > border_width / 2.0 {
                    *radius - border_width / 2.0
                } else {
                    is_simple_border = false;
                    0.0
                }
                .min(border_bounds.width / 2.0)
                .min(border_bounds.height / 2.0);
            }

            // Stroking a path works well in this case
            if is_simple_border {
                let border_path =
                    rounded_rectangle(border_bounds, border_radius);

                pixels.stroke_path(
                    &border_path,
                    &tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(into_color(
                            quad.border.color,
                        )),
                        anti_alias: true,
                        ..tiny_skia::Paint::default()
                    },
                    &tiny_skia::Stroke {
                        width: border_width,
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    clip_mask,
                );
            } else {
                // Draw corners that have too small border radii as having no border radius,
                // but mask them with the rounded rectangle with the correct border radius.
                let mut temp_pixmap = tiny_skia::Pixmap::new(
                    quad.bounds.width as u32,
                    quad.bounds.height as u32,
                )
                .unwrap();

                let mut quad_mask = tiny_skia::Mask::new(
                    quad.bounds.width as u32,
                    quad.bounds.height as u32,
                )
                .unwrap();

                let zero_bounds = Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: quad.bounds.width,
                    height: quad.bounds.height,
                };
                let path = rounded_rectangle(zero_bounds, fill_border_radius);

                quad_mask.fill_path(
                    &path,
                    tiny_skia::FillRule::EvenOdd,
                    true,
                    transform,
                );
                let path_bounds = Rectangle {
                    x: border_width / 2.0,
                    y: border_width / 2.0,
                    width: quad.bounds.width - border_width,
                    height: quad.bounds.height - border_width,
                };

                let border_radius_path =
                    rounded_rectangle(path_bounds, border_radius);

                temp_pixmap.stroke_path(
                    &border_radius_path,
                    &tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(into_color(
                            quad.border.color,
                        )),
                        anti_alias: true,
                        ..tiny_skia::Paint::default()
                    },
                    &tiny_skia::Stroke {
                        width: border_width,
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    Some(&quad_mask),
                );

                pixels.draw_pixmap(
                    quad.bounds.x as i32,
                    quad.bounds.y as i32,
                    temp_pixmap.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    transform,
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_text(
        &mut self,
        text: &Text,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match text {
            Text::Paragraph {
                paragraph,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) = clip_bounds
                    .intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, paragraph.min_bounds)
                        * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_paragraph(
                    paragraph,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Editor {
                editor,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) = clip_bounds
                    .intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, editor.bounds) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_editor(
                    editor,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Cached {
                content,
                bounds,
                color,
                size,
                line_height,
                font,
                align_x,
                align_y,
                shaping,
                clip_bounds: local_clip_bounds,
            } => {
                let physical_bounds = *local_clip_bounds * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_cached(
                    content,
                    *bounds,
                    *color,
                    *size,
                    *line_height,
                    *font,
                    *align_x,
                    *align_y,
                    *shaping,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Raw {
                raw,
                transformation: local_transformation,
            } => {
                let Some(buffer) = raw.buffer.upgrade() else {
                    return;
                };

                let transformation = transformation * *local_transformation;
                let (width, height) = buffer.size();

                let physical_bounds = Rectangle::new(
                    raw.position,
                    Size::new(
                        width.unwrap_or(clip_bounds.width),
                        height.unwrap_or(clip_bounds.height),
                    ),
                ) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                self.text_pipeline.draw_raw(
                    &buffer,
                    raw.position,
                    raw.color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
        }
    }

    pub fn draw_primitive(
        &mut self,
        primitive: &Primitive,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match primitive {
            Primitive::Fill { path, paint, rule } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x(),
                        y: bounds.y(),
                        width: bounds.width(),
                        height: bounds.height(),
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                pixels.fill_path(
                    path,
                    paint,
                    *rule,
                    into_transform(transformation),
                    clip_mask,
                );
            }
            Primitive::Stroke {
                path,
                paint,
                stroke,
            } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x() - stroke.width / 2.0,
                        y: bounds.y() - stroke.width / 2.0,
                        width: bounds.width() + stroke.width,
                        height: bounds.height() + stroke.width,
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                pixels.stroke_path(
                    path,
                    paint,
                    stroke,
                    into_transform(transformation),
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_image(
        &mut self,
        image: &Image,
        _transformation: Transformation,
        _pixels: &mut tiny_skia::PixmapMut<'_>,
        _clip_mask: &mut tiny_skia::Mask,
        _clip_bounds: Rectangle,
    ) {
        match image {
            #[cfg(feature = "image")]
            Image::Raster {
                image,
                bounds,
                clip_bounds,
            } => {
                let physical_bounds = *bounds * _transformation;
                // The image's own clip bounds (the widget's bounds for a
                // `ContentFit::Cover` image, which is larger than its box),
                // within the layer's.
                let Some(effective_clip) = (*clip_bounds * _transformation)
                    .intersection(&_clip_bounds)
                else {
                    return;
                };

                if !effective_clip.intersects(&physical_bounds) {
                    return;
                }

                // Like the image shader of wgpu, an image with a border
                // radius draws in its own clip bounds with rounded corners.
                let radii = <[f32; 4]>::from(image.border_radius);
                let rounded = radii.iter().any(|r| *r > 0.0);

                let needs_custom_mask = rounded
                    || (effective_clip != _clip_bounds
                        && !physical_bounds.is_within(&effective_clip));
                if needs_custom_mask {
                    adjust_clip_mask(_clip_mask, effective_clip);
                }
                if rounded {
                    let own = *clip_bounds * _transformation;
                    let max = own.width.min(own.height) / 2.0;
                    let radii = radii.map(|r| {
                        (r * _transformation.scale_factor()).min(max)
                    });
                    _clip_mask.intersect_path(
                        &rounded_rectangle(own, radii),
                        tiny_skia::FillRule::EvenOdd,
                        true,
                        tiny_skia::Transform::default(),
                    );
                }

                let clip_mask = (rounded
                    || !physical_bounds.is_within(&effective_clip))
                .then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(image.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.raster_pipeline.draw(
                    &image.handle,
                    image.filter_method,
                    *bounds,
                    image.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                );

                if needs_custom_mask {
                    adjust_clip_mask(_clip_mask, _clip_bounds);
                }
            }
            #[cfg(feature = "svg")]
            Image::Vector {
                svg,
                bounds,
                clip_bounds,
            } => {
                let physical_bounds = *bounds * _transformation;
                // The image's own clip bounds (the widget's bounds for a
                // `ContentFit::Cover` image, which is larger than its box),
                // within the layer's.
                let Some(effective_clip) = (*clip_bounds * _transformation)
                    .intersection(&_clip_bounds)
                else {
                    return;
                };

                if !effective_clip.intersects(&physical_bounds) {
                    return;
                }

                let needs_custom_mask = effective_clip != _clip_bounds
                    && !physical_bounds.is_within(&effective_clip);
                if needs_custom_mask {
                    adjust_clip_mask(_clip_mask, effective_clip);
                }

                let clip_mask = (!physical_bounds.is_within(&effective_clip))
                    .then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(svg.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.vector_pipeline.draw(
                    &svg.handle,
                    svg.color,
                    *bounds,
                    svg.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                );

                if needs_custom_mask {
                    adjust_clip_mask(_clip_mask, _clip_bounds);
                }
            }
            #[cfg(not(feature = "image"))]
            Image::Raster { .. } => {
                log::warn!(
                    "Unsupported primitive in `iced_tiny_skia`: {image:?}",
                );
            }
            #[cfg(not(feature = "svg"))]
            Image::Vector { .. } => {
                log::warn!(
                    "Unsupported primitive in `iced_tiny_skia`: {image:?}",
                );
            }
        }
    }

    pub fn trim(&mut self) {
        self.text_pipeline.trim_cache();

        #[cfg(feature = "image")]
        self.raster_pipeline.trim_cache();

        #[cfg(feature = "svg")]
        self.vector_pipeline.trim_cache();
    }
}

pub fn into_color(color: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(color.b, color.g, color.r, color.a)
        .expect("Convert color from iced to tiny_skia")
}

fn into_transform(transformation: Transformation) -> tiny_skia::Transform {
    let translation = transformation.translation();

    tiny_skia::Transform {
        sx: transformation.scale_factor(),
        kx: 0.0,
        ky: 0.0,
        sy: transformation.scale_factor(),
        tx: translation.x,
        ty: translation.y,
    }
}

/// beeper-rs: the colors of the rows of a quad whose fill is one color in
/// each row of pixels: a color, or a gradient straight down or up with a stop
/// at each end. Each is the pixel that tiny-skia's pipeline draws there.
enum Rows {
    Same([u8; 4]),
    /// The colors at the ends, and the place of a pixel between them from
    /// its row: `at + (row + 0.5) * step`, as tiny-skia computes it.
    Ramp { from: [f32; 4], to: [f32; 4], at: (f32, f32), step: f32 },
}

impl Rows {
    fn new(
        background: &Background,
        bounds: Rectangle,
        transformation: Transformation,
    ) -> Option<Self> {
        match background {
            Background::Color(color) => {
                let pixel = into_color(*color).premultiply().to_color_u8();

                Some(Self::Same([
                    pixel.red(),
                    pixel.green(),
                    pixel.blue(),
                    pixel.alpha(),
                ]))
            }
            Background::Gradient(Gradient::Linear(linear)) => {
                let (start, end) = linear.angle.to_distance(&bounds);
                let mut stops = linear.stops.into_iter().flatten();
                let (first, last) = (stops.next()?, stops.next()?);
                let (dx, dy) = (end.x - start.x, end.y - start.y);
                let ends = first.offset == 0.0 && last.offset == 1.0;

                if stops.next().is_some() || !ends || dx.abs() > dy.abs() * 1e-3 {
                    return None;
                }

                // The transform of tiny-skia's gradient, step by step: its
                // `points_to_unit_ts` after the inverse of the quad's.
                let inverse = 1.0 / (dx * dx + dy * dy).sqrt();
                let (sin, cos) = (-dy * inverse, dx * inverse);
                let unit = tiny_skia::Transform::from_row(
                    cos,
                    sin,
                    -sin,
                    cos,
                    sin * start.y + (1.0 - cos) * start.x,
                    -sin * start.x + (1.0 - cos) * start.y,
                )
                .post_translate(-start.x, -start.y)
                .post_scale(inverse, inverse);
                let ts = into_transform(transformation).invert()?.post_concat(unit);
                let middle = (bounds * transformation).center_x();
                let channels = |c: Color| [c.b, c.g, c.r, c.a];

                Some(Self::Ramp {
                    from: channels(first.color),
                    to: channels(last.color),
                    at: (middle * ts.sx, ts.tx),
                    step: ts.kx,
                })
            }
        }
    }

    /// The pixel of row `y`, from the middle of the row like the pipeline.
    fn at(&self, y: u32) -> [u8; 4] {
        match self {
            Self::Same(pixel) => *pixel,
            Self::Ramp { from, to, at, step } => {
                let t = at.0 + ((y as f32 + 0.5) * step + at.1);
                let t = t.clamp(0.0, 1.0);
                let [b, g, r, a] = std::array::from_fn(|i| {
                    let channel = t * (to[i] - from[i]) + from[i];

                    (channel.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
                });
                let faded = |c: u32| match from[3] < 1.0 || to[3] < 1.0 {
                    true => ((c * a + 255) >> 8) as u8,
                    false => c as u8,
                };

                [faded(b), faded(g), faded(r), a as u8]
            }
        }
    }
}

/// beeper-rs: fills a quad that is on whole pixels as plain rows of `rows`,
/// and only its corners as anti-aliased paths with `paint`. The path fill of
/// tiny-skia sends each span of the whole quad through its pipeline, which
/// was a quarter of a frame of the whole window for the card of the open
/// chat alone, and an eighth for the fade over its top. Returns whether it
/// filled the quad.
fn fill_rows(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    bounds: Rectangle,
    radii: [f32; 4],
    rows: &Rows,
    (paint, transformation): (&tiny_skia::Paint<'_>, Transformation),
    clip_bounds: Rectangle,
    clip_mask: Option<&tiny_skia::Mask>,
) -> bool {
    let whole = |r: Rectangle| {
        [r.x, r.y, r.width, r.height]
            .into_iter()
            .all(|v| (v - v.round()).abs() < 1e-3)
    };
    let corner = radii.into_iter().fold(0.0, f32::max).ceil();
    let clip = match clip_mask {
        None => Some(bounds),
        Some(_) if whole(clip_bounds) => bounds.intersection(&clip_bounds),
        Some(_) => None,
    };
    let Some(clip) = clip.filter(|_| {
        whole(bounds)
            && bounds.width * bounds.height >= 4096.0
            && bounds.width.min(bounds.height) >= 2.0 * corner
            && bounds.width.max(bounds.height) > 2.0 * corner
    }) else {
        return false;
    };

    let fill = |pixels: &mut tiny_skia::PixmapMut<'_>,
                x: f32,
                y: f32,
                width: f32,
                height: f32| {
        let part = Rectangle { x, y, width, height };
        let Some(part) = part.intersection(&clip).and_then(Rectangle::snap)
        else {
            return;
        };

        match rows {
            Rows::Same(pixel) => crate::blit::fill(pixels, part, *pixel),
            Rows::Ramp { .. } => {
                for y in part.y..part.y + part.height {
                    let row = Rectangle { y, height: 1, ..part };
                    crate::blit::fill(pixels, row, rows.at(y));
                }
            }
        }
    };
    let (right, bottom) = (
        bounds.x + bounds.width - corner,
        bounds.y + bounds.height - corner,
    );
    let (inner, between) =
        (bounds.width - 2.0 * corner, bounds.height - 2.0 * corner);

    fill(pixels, bounds.x, bounds.y + corner, bounds.width, between);
    fill(pixels, bounds.x + corner, bounds.y, inner, corner);
    fill(pixels, bounds.x + corner, bottom, inner, corner);

    let corners = [
        (bounds.x, bounds.y),
        (right, bounds.y),
        (right, bottom),
        (bounds.x, bottom),
    ];
    let scale = transformation.scale_factor();
    let moved = transformation.translation();

    for (i, (x, y)) in corners.into_iter().enumerate() {
        if radii[i] == 0.0 {
            fill(pixels, x, y, corner, corner);
            continue;
        }

        let part = Rectangle { x, y, width: corner, height: corner };
        let mut rounded = [0.0; 4];
        rounded[i] = radii[i] / scale;

        if part.intersects(&clip_bounds) {
            // The corner in the units of the quad, where a gradient is.
            let part = Rectangle {
                x: (x - moved.x) / scale,
                y: (y - moved.y) / scale,
                width: corner / scale,
                height: corner / scale,
            };

            pixels.fill_path(
                &rounded_rectangle(part, rounded),
                paint,
                tiny_skia::FillRule::EvenOdd,
                into_transform(transformation),
                clip_mask,
            );
        }
    }

    true
}

fn rounded_rectangle(
    bounds: Rectangle,
    border_radius: [f32; 4],
) -> tiny_skia::Path {
    let [top_left, top_right, bottom_right, bottom_left] = border_radius;

    if top_left == 0.0
        && top_right == 0.0
        && bottom_right == 0.0
        && bottom_left == 0.0
    {
        return tiny_skia::PathBuilder::from_rect(
            tiny_skia::Rect::from_xywh(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            )
            .expect("Build quad rectangle"),
        );
    }

    if top_left == top_right
        && top_left == bottom_right
        && top_left == bottom_left
        && top_left == bounds.width / 2.0
        && top_left == bounds.height / 2.0
    {
        return tiny_skia::PathBuilder::from_circle(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
            top_left,
        )
        .expect("Build circle path");
    }

    let mut builder = tiny_skia::PathBuilder::new();

    builder.move_to(bounds.x + top_left, bounds.y);
    builder.line_to(bounds.x + bounds.width - top_right, bounds.y);

    if top_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width - top_right,
            bounds.y,
            bounds.x + bounds.width,
            bounds.y + top_right,
            top_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bounds.width,
        bounds.y + bounds.height - bottom_right,
    );

    if bottom_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width,
            bounds.y + bounds.height - bottom_right,
            bounds.x + bounds.width - bottom_right,
            bounds.y + bounds.height,
            bottom_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bottom_left,
        bounds.y + bounds.height,
    );

    if bottom_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bottom_left,
            bounds.y + bounds.height,
            bounds.x,
            bounds.y + bounds.height - bottom_left,
            bottom_left,
        );
    }

    maybe_line_to(&mut builder, bounds.x, bounds.y + top_left);

    if top_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x,
            bounds.y + top_left,
            bounds.x + top_left,
            bounds.y,
            top_left,
        );
    }

    builder.finish().expect("Build rounded rectangle path")
}

fn maybe_line_to(path: &mut tiny_skia::PathBuilder, x: f32, y: f32) {
    if path.last_point() != Some(tiny_skia::Point { x, y }) {
        path.line_to(x, y);
    }
}

fn arc_to(
    path: &mut tiny_skia::PathBuilder,
    x_from: f32,
    y_from: f32,
    x_to: f32,
    y_to: f32,
    radius: f32,
) {
    let svg_arc = kurbo::SvgArc {
        from: kurbo::Point::new(f64::from(x_from), f64::from(y_from)),
        to: kurbo::Point::new(f64::from(x_to), f64::from(y_to)),
        radii: kurbo::Vec2::new(f64::from(radius), f64::from(radius)),
        x_rotation: 0.0,
        large_arc: false,
        sweep: true,
    };

    match kurbo::Arc::from_svg_arc(&svg_arc) {
        Some(arc) => {
            arc.to_cubic_beziers(0.1, |p1, p2, p| {
                path.cubic_to(
                    p1.x as f32,
                    p1.y as f32,
                    p2.x as f32,
                    p2.y as f32,
                    p.x as f32,
                    p.y as f32,
                );
            });
        }
        None => {
            path.line_to(x_to, y_to);
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let x = ((x - a) / (b - a)).clamp(0.0, 1.0);

    x * x * (3.0 - 2.0 * x)
}

fn rounded_box_sdf(
    to_center: Vector,
    size: tiny_skia::Size,
    radii: &[f32],
) -> f32 {
    let radius = match (to_center.x > 0.0, to_center.y > 0.0) {
        (true, true) => radii[2],
        (true, false) => radii[1],
        (false, true) => radii[3],
        (false, false) => radii[0],
    };

    let x = (to_center.x.abs() - size.width() + radius).max(0.0);
    let y = (to_center.y.abs() - size.height() + radius).max(0.0);

    (x.powf(2.0) + y.powf(2.0)).sqrt() - radius
}

pub fn adjust_clip_mask(clip_mask: &mut tiny_skia::Mask, bounds: Rectangle) {
    clip_mask.clear();

    let path = {
        let mut builder = tiny_skia::PathBuilder::new();
        builder.push_rect(
            tiny_skia::Rect::from_xywh(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            )
            .unwrap(),
        );

        builder.finish().unwrap()
    };

    clip_mask.fill_path(
        &path,
        tiny_skia::FillRule::EvenOdd,
        false,
        tiny_skia::Transform::default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Border, Point, Shadow};

    fn canvas() -> (tiny_skia::Pixmap, tiny_skia::Mask) {
        let mut pixels = tiny_skia::Pixmap::new(260, 200).unwrap();
        pixels.fill(tiny_skia::Color::from_rgba8(200, 120, 40, 255));
        (pixels, tiny_skia::Mask::new(260, 200).unwrap())
    }

    fn quad(shadow: Shadow) -> Quad {
        Quad {
            bounds: Rectangle::new(Point::new(20.0, 14.0), Size::new(90.0, 70.0)),
            border: Border { radius: crate::core::border::Radius { top_left: 10.0, top_right: 4.0, bottom_right: 0.0, bottom_left: 25.0 }, ..Border::default() },
            shadow,
            snap: false,
        }
    }

    /// A quad on whole pixels, filled as rows with path corners, is the path
    /// fill of tiny-skia pixel for pixel, also in a clip: an opaque color, a
    /// clear one, and gradients down and up with and without a clear end.
    #[test]
    fn quads_fill_in_rows_like_their_path() {
        use crate::core::gradient::Linear;
        use std::f32::consts::PI;

        let (blue, red) = (Color::from_rgb8(12, 80, 250), Color::from_rgba8(250, 60, 30, 0.37));
        let ramp = |angle: f32, from: Color, to: Color| {
            Background::Gradient(Gradient::Linear(Linear::new(angle).add_stop(0.0, from).add_stop(1.0, to)))
        };
        let fills = [
            Background::Color(blue),
            Background::Color(red),
            ramp(PI, blue, Color::WHITE),
            ramp(PI, Color::WHITE, Color::TRANSPARENT),
            ramp(0.0, red, blue),
        ];
        let quad = quad(Shadow::default());

        for (fill, (scale, clip)) in fills
            .iter()
            .flat_map(|f| [(1.0, None), (2.0, None), (2.0, Some((30.0, 20.0, 120.0, 150.0)))].map(|at| (f, at)))
        {
            let whole = Rectangle::with_size(Size::new(260.0, 200.0));
            let clip = clip.map_or(whole, |(x, y, w, h)| Rectangle::new(Point::new(x, y), Size::new(w, h)));
            let (mut fast, mut mask) = canvas();
            adjust_clip_mask(&mut mask, clip);
            Engine::new().draw_quad(&quad, fill, Transformation::scale(scale), &mut fast.as_mut(), &mut mask, clip);

            let shader = match fill {
                Background::Color(color) => tiny_skia::Shader::SolidColor(into_color(*color)),
                Background::Gradient(Gradient::Linear(linear)) => {
                    let (start, end) = linear.angle.to_distance(&quad.bounds);
                    let stops = linear.stops.into_iter().flatten().map(|stop| {
                        let c = stop.color;
                        tiny_skia::GradientStop::new(stop.offset, tiny_skia::Color::from_rgba(c.b, c.g, c.r, c.a).unwrap())
                    });
                    tiny_skia::LinearGradient::new(
                        tiny_skia::Point { x: start.x, y: start.y },
                        tiny_skia::Point { x: end.x, y: end.y },
                        stops.collect(),
                        tiny_skia::SpreadMode::Pad,
                        tiny_skia::Transform::identity(),
                    )
                    .unwrap()
                }
            };
            let (mut slow, _) = canvas();
            slow.as_mut().fill_path(
                &rounded_rectangle(quad.bounds, quad.border.radius.into()),
                &tiny_skia::Paint { shader, anti_alias: true, ..tiny_skia::Paint::default() },
                tiny_skia::FillRule::EvenOdd,
                tiny_skia::Transform::from_scale(scale, scale),
                (clip != whole).then_some(&mask),
            );
            let far = fast.data().iter().zip(slow.data()).map(|(a, b)| a.abs_diff(*b)).max();
            assert_eq!(far, Some(0), "{fill:?} at scale {scale} in {clip:?}");
        }
    }

    /// The shadow of a quad, computed once for each row and column beside
    /// its straight sides, is the shadow computed for each pixel, within a
    /// step of each channel.
    #[test]
    fn shadows_repeat_beside_straight_sides() {
        let shadow = Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.5),
            offset: Vector::new(2.0, 3.0),
            blur_radius: 12.0,
        };
        let (quad, scale) = (quad(shadow), 2.0);
        let clip = Rectangle::with_size(Size::new(260.0, 200.0));
        let (mut fast, mut mask) = canvas();
        adjust_clip_mask(&mut mask, clip);
        // A clear fill, so the shadow shows under the whole quad.
        Engine::new().draw_quad(
            &quad,
            &Background::Color(Color::TRANSPARENT),
            Transformation::scale(scale),
            &mut fast.as_mut(),
            &mut mask,
            clip,
        );

        let bounds = quad.bounds * Transformation::scale(scale);
        let half = tiny_skia::Size::from_wh(bounds.width / 2.0, bounds.height / 2.0).unwrap();
        let radii: Vec<f32> = <[f32; 4]>::from(quad.border.radius).iter().map(|r| r * scale).collect();
        let (slow, _) = canvas();
        for (i, (got, under)) in fast.pixels().iter().zip(slow.pixels()).enumerate() {
            let (x, y) = ((i % 260) as f32, (i / 260) as f32);
            let to_center = Vector::new(
                x - bounds.x - shadow.offset.x * scale - half.width(),
                y - bounds.y - shadow.offset.y * scale - half.height(),
            );
            let distance = rounded_box_sdf(to_center, half, &radii).max(0.0);
            let alpha = 0.5 * (1.0 - smoothstep(-shadow.blur_radius * scale, shadow.blur_radius * scale, distance));
            // Black at `alpha` over the canvas, inside the box the shadow is drawn in.
            let inside = (bounds.x + 4.0 - 24.0..bounds.x + 4.0 + bounds.width + 24.0).contains(&x)
                && (bounds.y + 6.0 - 24.0..bounds.y + 6.0 + bounds.height + 24.0).contains(&y);
            let alpha = if inside { alpha } else { 0.0 };
            for (got, under) in [(got.red(), under.red()), (got.green(), under.green()), (got.blue(), under.blue())] {
                let want = f32::from(under) * (1.0 - alpha);
                assert!((f32::from(got) - want).abs() <= 1.6, "{got} for {want} at {x}, {y}");
            }
        }
    }
}
