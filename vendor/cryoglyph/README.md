This directory is cryoglyph 0.1.0 from crates.io, without its examples and benchmarks, with these changes:

- `TextRenderer::reuse` keeps what the last `prepare` prepared for one more frame when no atlas has evicted a glyph
  since (a generation that atlas creation and every eviction change): it marks the prepared glyphs in use, so that
  the rest of the frame cannot evict them. iced_wgpu's text pipeline calls it for a group whose text did not change.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml together with vendor/iced_wgpu's text
reuse, or when a cryoglyph release can keep prepared text.
