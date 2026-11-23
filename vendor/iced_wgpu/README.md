This directory is iced_wgpu 0.14.0 from crates.io with these changes:

- The SVG cache parses SVGs with an empty font database instead of loading the system fonts for each renderer, which
  is each window, since the app's SVGs have no text.
- A vector's color filter keeps the alpha of the color, so a translucent icon is not opaque.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when an iced release reuses one font
database or skips it for SVGs without text.
