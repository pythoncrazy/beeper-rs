This directory is iced_graphics 0.14.0 from crates.io with these changes:

- `text::to_attributes` reads a family name `Name` followed by `+tag` and `@em` parts, like `Inter+tnum@0.02`, as
  the family `Name` with the OpenType feature `tag` on (`Attrs::font_features`; CSS `font-variant-numeric:
  tabular-nums` is `+tnum`) and a letter spacing of `em` (`Attrs::letter_spacing`), which cosmic-text applies when it
  shapes. iced's `Font` has no fields for them, and every text of iced (paragraphs, the text cache, editors and
  inputs, on both renderers) gets its attributes here, while the name is part of the font's equality and hash that
  the caches use. beeper-ui's `style::spaced` and `style::tabular` make such fonts.
- `Editor::update` centers every line of an editor whose font's family name has a `#center` part (`text::centered`;
  CSS `text-align: center` of a `textarea`), which iced's text editor has no setting for: beeper-ui's
  `style::centered`. The lines that edits make are centered too.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when an iced release can give a text a
letter spacing and font features, and center the lines of a text editor.
