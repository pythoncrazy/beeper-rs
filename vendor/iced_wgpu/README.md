This directory is iced_wgpu 0.14.0 from crates.io with these changes:

- The SVG cache parses SVGs with an empty font database instead of loading the system fonts for each renderer, which
  is each window, since the app's SVGs have no text.
- A vector's color filter keeps the alpha of the color, so a translucent icon is not opaque.
- A text group that draws the same paragraphs at the same places as the frame before keeps its prepared glyphs
  (`text::Prepared`, with the vendored cryoglyph's `TextRenderer::reuse`) instead of laying out, looking up and uploading
  every glyph again in every frame.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when an iced release reuses one font
database or skips it for SVGs without text, and keeps unchanged text between frames.
