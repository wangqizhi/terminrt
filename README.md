# TerminRT

A GPU-accelerated terminal emulator built with Rust, combining the Alacritty terminal emulation engine with an egui-based GUI rendered via WebGPU (wgpu).

## Features

- **VT100 Terminal Emulation** — Full ANSI escape sequence support powered by `alacritty_terminal`
- **GPU-Accelerated Rendering** — Custom WGSL shaders with dual render pipelines (color + glyph) via `wgpu`
- **Windows ConPTY Integration** — Spawns PowerShell sessions through the Windows ConPTY API
- **Font Rasterization** — System font loading and glyph rendering with `fontdue`
- **Text Selection & Clipboard** — Mouse-based text selection with copy support (up to 2MB)
- **IME Support** — Input Method Editor cursor position reporting for CJK input
- **Bracketed Paste Mode** — Proper paste handling for terminal applications
- **DevTools Panel** — Collapsible panel displaying raw VT stream output for debugging
- **OSC Sequence Parsing** — Tracks current working directory via `OSC 633` sequences from PowerShell
- **Startup Animation** — Animated loading screen with initialization status
- **Close Confirmation Dialog** — Prevents accidental window closure
- **Cursor Blinking** — 500ms on/off blinking cursor animation
- **ANSI Colors** — Full 256-color palette (16 base + 216 color cube + 24 grayscale)
- **Scrollback** — Keyboard-driven scrolling with Ctrl+L screen reset

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (2021 edition or later)
- Windows 10/11 (ConPTY support required)
- A GPU with WebGPU/Vulkan/DX12 support

## Build & Run

```bash
# Clone the repository
git clone <repo-url>
cd lib-terminal-rt

# Build in release mode
cargo build --release

# Run
cargo run --release
```

The compiled binary will be located at `target/release/terminrt.exe`.

## Key Dependencies

| Crate | Purpose |
|---|---|
| `winit` 0.29 | Window creation and event handling |
| `wgpu` 0.19 | GPU rendering (WebGPU backend) |
| `egui` 0.27 | Immediate-mode GUI framework |
| `alacritty_terminal` 0.25 | VT100 terminal emulation |
| `conpty` 0.7 | Windows ConPTY API bindings |
| `fontdue` 0.8 | Font rasterization |
| `arboard` 3.6 | Clipboard access |

## Architecture

```
src/
├── main.rs          # Event loop, GPU setup, UI layout, rendering
├── terminal.rs      # Terminal state, color mapping, selection, scrolling
├── pty.rs           # PTY abstraction (ConPTY on Windows)
├── input.rs         # Input command parsing
├── font.rs          # Font loading and glyph rasterization
├── startup-page.rs  # Loading animation UI
└── shader.wgsl      # WebGPU vertex/fragment shaders
```

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+L` | Scroll to screen top |
| `Alt+F4` | Close (with confirmation) |

## License

MIT License
