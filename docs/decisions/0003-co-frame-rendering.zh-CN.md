# ADR 0003 — 终端与 egui 同帧渲染

> English (default): [`0003-co-frame-rendering.md`](0003-co-frame-rendering.md)

状态:已接受。

## 背景

周边 UI(面板、设置)想用 egui,终端网格要自绘 GPU,二者要在同一窗口、不抢 swapchain。

## 决定

- **egui 驱动整帧**(eframe + wgpu)。终端在同一 `wgpu::RenderPass` 里通过
  **`egui_wgpu` 的 `PaintCallback`** 绘制。
- 字形管线放在 `term-render`(wgpu + glyphon),存进 egui 的 `callback_resources`;
  与 egui 共享同一 `Device`/`Queue`/`Surface`(同一 `wgpu` 版本——当前 23)。
- 背景/选区/光标由 egui painter 画在**下层**;回调把字形画在上层。

## 后果

- 单个 pass、单个 device;没有第二个 surface 或 swapchain。
- `wgpu` 必须与 egui-wgpu 的版本锁定一致(不一致就无法共享 device)——所以 `glyphon`
  锁在兼容 wgpu 23 的版本(0.7)。
- 每个 pane 拥有自己的 `TermRenderer`(各自的字形图集),因此保留已 prepare 的字形;
  仅当该 pane 内容变脏(damage)时才重新 shape,空闲帧不做文字工作。
- Update:对原生宿主,本决定已被 ADR 0030 取代 —— 网格由 `term-widget` 的原生
  `winit` + `wgpu` 循环绘制,而非 egui `PaintCallback`。共帧渲染仍适用于 `eframe` 宿主。
