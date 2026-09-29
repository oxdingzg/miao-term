# ADR 0029 — 原生渲染循环（自绘网格）

> English (default): [`0030-native-render-loop.md`](0030-native-render-loop.md)

状态:已接受。

## 背景

`miaotty-app` 最初基于 `eframe`/`egui` 引导:终端网格通过 egui 的 `PaintCallback`
绘制,所有交互都走 egui 的 immediate-mode 帧循环。开发快,但输入到上屏的延迟被框架的
事件循环 + present 节拍卡住,明显慢于原生终端(Otty / Ghostty / Alacritty)。

引擎本就是为此设计的:`term-core` 拥有热路径,不依赖 GPU 与窗口;`term-render` 拥有
字体/图集/绘制通道,不含事件循环(见 ARCHITECTURE §3、D3)。

## 决定

把终端网格迁移到由 `term-widget` 拥有的**原生 `winit` + `wgpu` 渲染循环**,不再经过 egui:

- `term-widget` 拥有 `winit` 事件循环与 `wgpu` surface。
- `term-render` 拥有各绘制通道:背景/选区/光标四边形 + 字形,直接画进 surface(不经 egui 网格)。
- 渲染循环是**事件驱动 + 按需重绘**:PTY 读取线程一有输出就通过 `EventLoopProxy`
  唤醒循环(`request_redraw`),回显下一帧即出;空闲 ≈ 0。
- egui 仍可用于周边 UI(面板/设置),在**同一帧**内合成(D3);但网格不依赖它。
- 呈现模式选择低延迟(`AutoNoVsync`),`desired_maximum_frame_latency = 1`。

## 后果

- 从 PTY 输出到上屏只差一帧 → 延迟接近原生终端;网格与 egui 帧节奏解耦。
- `term-widget` 成长为一个真正的宿主:窗口、surface、输入、IME、剪贴板、resize、重绘调度。
- `term-render` 在字形渲染器之外新增四边形通道(`QuadRenderer`)。
- 现有 `eframe` 应用继续作为功能完整的界面宿主,直到原生宿主达到功能对等;
  两者共享同一套引擎 crate。
