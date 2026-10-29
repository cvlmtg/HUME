local wezterm = require("wezterm")
-- The terminal record.sh films the demo in, passed with --config-file so your
-- own WezTerm config is never read.
return {
  initial_cols = 110,
  initial_rows = 32,
  font_size = 15,
  enable_tab_bar = false,
  force_reverse_video_cursor = true,
  font = wezterm.font("Fira Code"),
  window_decorations = "MACOS_USE_BACKGROUND_COLOR_AS_TITLEBAR_COLOR|TITLE|RESIZE",
  window_padding = { left = 0, right = 0, top = 0, bottom = 0 },
  window_close_confirmation = 'NeverPrompt',
  quit_when_all_windows_are_closed = true,
  check_for_updates = false,
  audible_bell = 'Disabled',
  term = "xterm-256color",
}
