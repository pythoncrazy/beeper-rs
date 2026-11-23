This directory is iced_winit 0.14.1 from crates.io with this change:

- `window::Action::Close` keeps the compositor when the last window closes. The app hides its windows on Wayland by
  closing them and opening them again under the same ID (winit cannot hide a window there), and a new compositor
  first loads the GPU driver again, which takes up to seconds, so the window would show late.

Delete this directory and its [patch.crates-io] entry in the root Cargo.toml when iced keeps the compositor, or when
winit can hide a window on Wayland.
