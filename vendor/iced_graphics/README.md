This directory is iced_graphics 0.14.0 from crates.io with this change:

- `text::to_attributes` reads a family name `Name@em` as the family `Name` with a letter spacing of `em`, which
  cosmic-text applies to every glyph when it shapes (`Attrs::letter_spacing`). iced's `Font` has no field for a
  letter spacing, and every text of iced (paragraphs, the text cache, editors and inputs, on both renderers) gets
  its attributes here, while the name is part of the font's equality and hash that the caches use.
  beeper-ui's `style::spaced` makes such fonts.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when an iced release can give a text a
letter spacing.
