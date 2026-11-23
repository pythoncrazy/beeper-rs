This directory is iced_winit 0.14.1 from crates.io with these changes:

- `window::Action::Close` keeps the compositor when the last window closes. The app hides its windows on Wayland by
  closing them and opening them again under the same ID (winit cannot hide a window there), and a new compositor
  first loads the GPU driver again, which takes up to seconds, so the window would show late.
- `window::resize` takes a size that winit applies at once, as it does on Wayland, where no `Resized` event follows:
  the window lays out and draws at the new size, instead of stretching its old frame over the new one until the next
  configure.
- `Cargo.toml` allows the `deprecated` lint, which the crate's `try_next` calls hit with the locked futures, since a
  path dependency's warnings show in every build.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when iced keeps the compositor (or winit
can hide a window on Wayland) and an iced release handles the size that `request_inner_size` returns.
