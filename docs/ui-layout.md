# UI 布局文档 / UI Layout Documentation

本文档描述了 lib-terminal-rt 主界面的所有 UI 区域及其功能。

## 整体布局概览

```
+------------------+--------------------------------------+------------------+
|                  |          Prompt Bar / 顶部状态栏       |                  |
|                  +--------------------------------------+                  |
|                  |              ↓ 渐变过渡                |                  |
|   Left Panel     |                                      |   Right Panel    |
|   左面板/侧边栏   |      Terminal Area / PTY 渲染区域      |   右面板/DevTools |
|                  |                                      |                  |
|                  |              ↑ 渐变过渡                |                  |
|                  +--------------------------------------+                  |
|                  |         Bottom Bar / 底部信息栏        |                  |
+------------------+--------------------------------------+------------------+
```

---

## 区域详细说明

### 1. Left Panel / 左面板 / 侧边栏

| 属性 | 值 |
|------|-----|
| 代码标识 | `"left_panel"` (SidePanel ID) |
| 类型 | `egui::SidePanel::left` |
| 代码位置 | [main.rs:843](src/main.rs#L843) |
| 宽度 | 260px 固定 (`LEFT_PANEL_WIDTH` 常量, [main.rs:26](src/main.rs#L26)) |
| 背景色 | `Color32::from_gray(18)` |
| 可调整大小 | 否 |

**功能：** 左侧边栏，包含 DevTools 面板的开关按钮。按钮文字为 `"DevTools ◀"` (面板打开时) 或 `"DevTools ▶"` (面板关闭时)，布局为底部对齐 (`Layout::bottom_up`)。

---

### 2. Right Panel / 右面板 / DevTools 面板

| 属性 | 值 |
|------|-----|
| 代码标识 | `"right_panel"` (SidePanel ID) |
| 类型 | `egui::SidePanel::right` |
| 代码位置 | [main.rs:866](src/main.rs#L866) |
| 宽度 | 屏幕总宽度的 25%（打开时），0（关闭时） |
| 背景色 | `Color32::from_gray(18)` |
| 可调整大小 | 否 |
| 显示条件 | `ui_state.devtools_open == true` |

**功能：** DevTools 调试面板，显示 VT (Virtual Terminal) 转义序列流日志。内部包含：
- 标题标签 `"VT Stream"`（等宽字体，灰色文字）
- 可滚动的 VT 日志区域（由 `render_vt_log` 函数渲染）

---

### 3. Central Panel / 中央面板 / 主内容区

| 属性 | 值 |
|------|-----|
| 代码标识 | `"CentralPanel"` |
| 类型 | `egui::CentralPanel::default` |
| 代码位置 | [main.rs:884](src/main.rs#L884) |
| 大小 | 填充左右面板之间的剩余空间 |
| 背景色 | `Color32::from_gray(20)` |

**功能：** 主内容容器，占据左右面板之间的全部空间。内部通过 `allocate_ui_at_rect` 划分为三个子区域：Prompt Bar、Terminal Area、Bottom Bar。

---

### 4. Prompt Bar / 顶部状态栏 / 提示栏

| 属性 | 值 |
|------|-----|
| 代码标识 | `prompt_rect` |
| 类型 | 通过 `allocate_ui_at_rect` 分配的矩形区域 |
| 代码位置 | [main.rs:903](src/main.rs#L903), [main.rs:915-950](src/main.rs#L915-L950) |
| 高度 | 22px (`bar_h`) |
| 背景色 | `Color32::from_gray(26)` |
| 渐变效果 | 向下 30px 渐变过渡到透明 |

**功能：** 顶部状态区域，在终端 Shell 退出时显示重连控件：
- 终端退出时：显示 `"PowerShell exited"` 文字 + `"Reconnect"` 按钮
- 正在重连时：显示 `"Reconnecting..."` 状态文字

---

### 5. Terminal Area / PTY 渲染区域 / 终端显示区

| 属性 | 值 |
|------|-----|
| 代码标识 | `terminal_rect` |
| 类型 | 通过 `allocate_ui_at_rect` 分配的矩形区域 |
| 代码位置 | [main.rs:905-908](src/main.rs#L905-L908), [main.rs:953-1027](src/main.rs#L953-L1027) |
| 大小 | 宽度：可用宽度 - 8px 左边距；高度：剩余垂直空间 |
| 背景色 | `Color32::from_rgb(18, 18, 18)` |
| 内边距 | 左 8px，上下各 14px |

**功能：** 核心区域，渲染 PTY 终端内容。内部包含一个垂直滚动区域 (`egui::ScrollArea::vertical`)，用于显示：
- 终端文本网格（单元格、颜色、样式）
- 光标渲染
- 鼠标文本选择高亮
- 支持滚动请求：`ScreenTop`（滚动到顶部）、`CursorTop`（光标到顶部）、`CursorLine`（光标行）

滚动区域代码标识：`("terminal_scroll", scroll_id)`，位于 [terminal.rs:533](src/terminal.rs#L533)。

---

### 6. Bottom Bar / 底部信息栏 / 底部状态栏

| 属性 | 值 |
|------|-----|
| 代码标识 | `bottom_rect` |
| 类型 | 通过 `allocate_ui_at_rect` 分配的矩形区域 |
| 代码位置 | [main.rs:909-912](src/main.rs#L909-L912), [main.rs:1072-1130](src/main.rs#L1072-L1130) |
| 高度 | 22px (`bar_h`) |
| 背景色 | `Color32::from_gray(26)` |
| 文字颜色 | `Color32::from_gray(120)` |
| 渐变效果 | 从透明向上 30px 渐变过渡到实色 |

**功能：** 底部信息展示栏，显示终端运行时状态，格式为：

```
Terminal: {status} | View: {x}x{y}px | PTY: {x}x{y}px ({cols}x{rows} cells)
```

包含以下信息：
- **连接状态** (`status`)：`connected` / `reconnecting` / `exited` / `failed` / `starting`
- **视图尺寸** (`View`)：终端显示区域的像素大小
- **PTY 渲染尺寸** (`PTY`)：PTY 渲染区的像素大小
- **网格大小**：PTY 的列数和行数（如 `80x24 cells`）

---

### 7. VT Log Scroll Area / VT 日志滚动区

| 属性 | 值 |
|------|-----|
| 代码标识 | `render_vt_log` 函数 |
| 类型 | `egui::ScrollArea::both` |
| 代码位置 | [terminal.rs:860-895](src/terminal.rs#L860-L895) |
| 字体 | 等宽字体，12pt |
| 文字颜色 | `Color32::from_gray(170)` |
| 最大行数 | 2000 (`VT_LOG_MAX_LINES`, [terminal.rs:20](src/terminal.rs#L20)) |

**功能：** 位于 Right Panel 内部，双向可滚动区域，显示 VT 转义序列日志。支持虚拟化渲染（`show_rows`），自动粘底（`stick_to_bottom: true`）以始终展示最新条目。

---

### 8. Close Confirm Dialog / 关闭确认对话框

| 属性 | 值 |
|------|-----|
| 代码标识 | `"close_confirm_dialog"` |
| 类型 | `egui::Window`（模态窗口） |
| 代码位置 | [main.rs:726-812](src/main.rs#L726-L812) |
| 大小 | 270 x 130 px 固定 |
| 位置 | 屏幕居中 |
| 背景色 | `Color32::from_rgb(24, 24, 24)` |
| 边框 | 1px `Color32::from_gray(70)`，圆角 8px |
| 遮罩层 | `Color32::from_rgba_unmultiplied(0, 0, 0, 70)` 半透明黑色 |
| 显示条件 | `ui_state.close_confirm_open == true` |

**功能：** 用户尝试关闭窗口时弹出的模态确认对话框：
- 标题：`"Confirm Close"`
- 正文：`"Are you sure you want to close this window?"`
- 副文：`"Your current terminal session will be interrupted."`
- 按钮：`"Close"`（蓝色, RGB 45/125/235）和 `"Cancel"`，各 92x30px

---

### 9. Startup Page / 启动页面 / 加载动画页

| 属性 | 值 |
|------|-----|
| 代码标识 | 启动页（terminal 为 None 时渲染） |
| 类型 | 自定义渲染内容 |
| 代码位置 | [startup-page.rs:20-77](src/startup-page.rs#L20-L77) |
| 背景色 | `Color32::from_rgb(14, 14, 14)` |
| 动画时长 | 约 2.3 秒 |

**功能：** 终端初始化期间显示的加载动画页：
- 动画文字：`"HELLO TERMINRT!"`，逐字符淡入效果（每字 0.12s 间隔，淡入 0.26s）
- 状态信息：`"Initializing terminal... dev by wqz"`
- 如果初始化失败：显示 `"PTY start failed: {error message}"`

---

## 渲染层级 / Rendering Layers

界面使用 egui 的 Layer 系统来控制渲染顺序：

| 层级 | 代码标识 | 渲染顺序 | 用途 |
|------|---------|---------|------|
| 模态遮罩层 | `"close_confirm_modal_blocker"` | Middle | 关闭确认对话框背后的半透明遮罩 |
| 渐变覆盖层 | `"gradient_overlays"` | Foreground | 顶部/底部状态栏的渐变过渡效果 |
| 文字覆盖层 | `"overlay_text"` | Tooltip | 底部状态栏文字（渲染在渐变之上） |

---

## 关键常量 / Key Constants

| 常量 | 值 | 说明 | 位置 |
|------|-----|------|------|
| `WINDOW_WIDTH` | 1638 | 窗口初始宽度 (px) | [main.rs:22](src/main.rs#L22) |
| `WINDOW_HEIGHT` | 1024 | 窗口初始高度 (px) | [main.rs:23](src/main.rs#L23) |
| `LEFT_PANEL_WIDTH` | 260.0 | 左面板固定宽度 (px) | [main.rs:26](src/main.rs#L26) |
| `TERM_FONT_SIZE` | 14.0 | 终端文字字号 (pt) | [terminal.rs:19](src/terminal.rs#L19) |
| `bar_h` | 22.0 | 顶部/底部栏高度 (px) | [main.rs:891](src/main.rs#L891) |
| `bar_pad` | 14.0 | 状态栏与终端区域间距 (px) | [main.rs:892](src/main.rs#L892) |
| `bar_fade` | 30.0 | 渐变过渡长度 (px) | [main.rs:893](src/main.rs#L893) |
| `VT_LOG_MAX_LINES` | 2000 | VT 日志最大行数 | [terminal.rs:20](src/terminal.rs#L20) |

---

## 相关 UI 状态 / UI State

来自 `UiState` 结构体（[main.rs:27-47](src/main.rs#L27-L47)）：

| 字段 | 类型 | 控制的 UI 区域 |
|------|------|--------------|
| `devtools_open` | `bool` | Right Panel 的显示/隐藏 |
| `terminal_view_size_px` | `egui::Vec2` | 终端显示区域尺寸 |
| `pty_render_size_px` | `egui::Vec2` | PTY 渲染尺寸 |
| `pty_grid_size` | `(usize, usize)` | PTY 网格大小 (cols, rows) |
| `close_confirm_open` | `bool` | Close Confirm Dialog 的显示/隐藏 |
| `terminal_exited` | `bool` | Prompt Bar 重连控件的显示 |
| `terminal_connecting` | `bool` | 重连中状态的显示 |
| `terminal_scroll_request` | `Option<ScrollRequest>` | 终端滚动区域的滚动行为 |
| `terminal_scroll_id` | `u64` | 滚动区域 ID（Ctrl+L 重置用） |
